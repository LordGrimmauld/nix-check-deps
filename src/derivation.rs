use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use ignore::Walk;
use lddtree::DependencyAnalyzer;
use log::{debug, error, warn};
use once_cell::sync::OnceCell;
use pyproject_toml::PyProjectToml;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::{self, File},
    path::PathBuf,
    str::FromStr,
};
use tar::Archive;
use tempfile::TempDir;
use xz::read::XzDecoder;

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::UTF8, BinaryDetection, Searcher},
};

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

use crate::get_nix_flags;

#[derive(Deserialize, Hash, Eq, PartialEq, Debug, Clone)]
struct DrvOutput {
    path: String,
}

impl DrvOutput {
    fn path(&self) -> String {
        self.path.clone()
    }
}

#[derive(Deserialize, Hash, Eq, PartialEq, Debug, Clone)]
struct DrvInput {
    outputs: Vec<String>,
}

#[derive(Deserialize, Hash, Eq, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct DrvEnv {
    #[serde(default)]
    build_inputs: Option<String>,
    #[serde(default)]
    check_inputs: Option<String>,
    #[serde(default)]
    pname: Option<String>,
    #[serde(default)]
    propagated_build_inputs: Option<String>,
    src: Option<String>,
}

impl DrvEnv {
    fn get_build_inputs(&self) -> Vec<String> {
        self.build_inputs.as_ref().map_or_else(Vec::new, |s| {
            s.split_whitespace().map(str::to_owned).collect()
        })
    }

    fn get_check_inputs(&self) -> Vec<String> {
        self.check_inputs.as_ref().map_or_else(Vec::new, |s| {
            s.split_whitespace().map(str::to_owned).collect()
        })
    }
    fn get_propagated_build_inputs(&self) -> Vec<String> {
        self.propagated_build_inputs
            .as_ref()
            .map_or_else(Vec::new, |s| {
                s.split_whitespace().map(str::to_string).collect()
            })
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Derivation {
    env: DrvEnv,
    outputs: HashMap<String, DrvOutput>,
    input_drvs: HashMap<String, DrvInput>,
    #[serde(skip_deserializing)]
    parsed_input_drvs: OnceCell<Vec<Derivation>>,
    #[serde(skip_deserializing)]
    pub drv_path: String,
    #[serde(skip_deserializing)]
    extracted_src_archive: OnceCell<Option<TempDir>>,
    #[serde(skip_deserializing)]
    build_outputs: OnceCell<Result<Vec<String>, std::io::Error>>,
}

impl Derivation {
    pub fn read_drv(drv_path: &str) -> Option<Self> {
        let drv_path = if drv_path.ends_with(".drv") {
            &format!("{}^*", drv_path)
        } else {
            drv_path
        };
        let output = Command::new("nix")
            .arg("derivation")
            .arg("show")
            .arg(drv_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .ok()?;
        let drvs: HashMap<String, Derivation> = serde_json::from_reader(output.stdout?)
            .inspect_err(|e| {
                error!("Can not parse drv {}: {}", drv_path, e);
            })
            .ok()?; // FIXME ?
        drvs.into_iter().last().map(|(path, mut drv)| {
            drv.drv_path = path.to_owned();
            drv
        })
    }

    fn get_input_drv_paths(&self) -> Vec<String> {
        self.input_drvs.clone().into_keys().collect()
    }

    fn get_src_drv(&self) -> Option<&Derivation> {
        let src_drv_path = self.env.src.as_ref()?;
        self.get_inputs()
            .iter()
            .find(|i| i.get_out_paths().contains(src_drv_path))
    }

    fn read_src_dir(&self) -> Option<PathBuf> {
        // TODO: maybe integrate with https://github.com/milahu/nix-build-debug or similar
        let src = self.get_src_drv()?;
        let build_results = src.build().as_ref().ok()?;

        let src_archive_path = PathBuf::from(build_results.first()?);
        if !src_archive_path.exists() {
            return None;
        }
        if src_archive_path.is_dir() {
            return Some(src_archive_path);
        }

        self.extracted_src_archive
            .get_or_init(|| try_extract_source_archive(src_archive_path))
            .as_ref()
            .map(|t| t.path().to_path_buf())
    }

    fn get_inputs(&self) -> &Vec<Derivation> {
        self.parsed_input_drvs.get_or_init(|| {
            self.input_drvs
                .keys()
                .flat_map(|p| Derivation::read_drv(p).into_iter())
                .collect()
        })
    }

    pub fn get_out_paths(&self) -> Vec<String> {
        let mut outputs: Vec<String> = self.outputs.values().map(DrvOutput::path).collect();

        if let Some(pname) = &self.env.pname {
            outputs.extend(
                self.get_inputs()
                    .iter()
                    .filter(|d| d.matches_pname(pname))
                    .flat_map(|d| d.get_out_paths()),
            );
        }

        outputs
    }

    pub fn read_deps(&self) -> Vec<Derivation> {
        let dev_inputs: Vec<String> = self.env.get_build_inputs();

        let mut dep_relations: Vec<Derivation> = Vec::new();
        let mut propagated: Vec<String> = Vec::new();
        let check_inputs = self.env.get_check_inputs();

        let all_inputs = self.get_input_drv_paths();
        for dep_drv in all_inputs
            .iter()
            .flat_map(|dep| Derivation::read_drv(dep).into_iter())
        {
            let propagated_drvs = dep_drv.env.get_propagated_build_inputs();
            let outputs: Vec<String> = dep_drv.get_out_paths();

            if outputs.iter().any(|o| dev_inputs.contains(o)) {
                dep_relations.push(dep_drv);
            }
            propagated.append(&mut propagated_drvs.clone());
        }

        dep_relations.retain(|dep_drv| {
            !propagated
                .iter()
                .any(|p| dep_drv.get_out_paths().contains(p))
        });
        dep_relations.retain(|dep_drv| {
            !check_inputs
                .iter()
                .any(|p| dep_drv.get_out_paths().contains(p))
        });
        dep_relations
    }

    pub fn matches_pname(&self, pname: &str) -> bool {
        self.env.pname.as_ref().is_some_and(|p| p == pname)
    }

    pub fn find_used_pyproject_deps(&self) -> HashSet<String> {
        let src_dir = if let Some(src_dir) = self.read_src_dir() {
            src_dir
        } else {
            return HashSet::new();
        };

        let mut src_dir = src_dir.clone();
        src_dir.push("pyproject.toml");

        if !src_dir.try_exists().unwrap_or(false) {
            return HashSet::new();
        }

        let pyproj = if let Some(pyproj) = fs::read_to_string(src_dir)
            .ok()
            .and_then(|f| PyProjectToml::new(&f).ok())
        {
            pyproj
        } else {
            return HashSet::new();
        };

        if let Some(proj) = pyproj.project {
            let req_deps = proj.dependencies.into_iter().flat_map(|v| v.into_iter());
            let opt_deps = proj
                .optional_dependencies
                .into_iter()
                .flat_map(|v| v.into_values())
                .flat_map(|f| f.into_iter());
            let deps: HashSet<String> = opt_deps
                .chain(req_deps)
                .map(|r| r.name.to_string())
                .collect();
            return deps;
        }

        HashSet::new()
    }

    pub fn find_used_shebangs(&self) -> HashSet<String> {
        let src_dir = if let Some(src_dir) = self.read_src_dir() {
            src_dir
        } else {
            return HashSet::new();
        };

        let mut shebangs = HashSet::new();
        let shebang_regex_str = r"^#! *\/((nix\/store\/.*\/)?(usr\/)?)bin\/((env +)?([^\s]+))";
        let shebang_regex = Regex::new(shebang_regex_str).unwrap();
        for e in Walk::new(&src_dir).flat_map(Result::into_iter) {
            let is_dir = fs::canonicalize(e.path()).ok().is_some_and(|p| p.is_dir());
            if is_dir {
                continue;
            }

            if let Ok(file) = File::open(e.path()) {
                let mut line = String::new();
                if BufReader::new(file).read_line(&mut line).is_ok() {
                    if let Some(program) = shebang_regex.captures(&line).and_then(|c| c.get(6)) {
                        debug!(
                            "{} uses shebang program: {}",
                            e.path().display(),
                            program.as_str()
                        );
                        shebangs.insert(program.as_str().to_string());
                    }
                }
            }
        }
        // println!("{:?}", shebangs);
        shebangs
    }

    pub fn find_used_shared_objects(&self) -> HashSet<PathBuf> {
        let mut shared_objects = HashSet::new();
        for out in self.build().iter().flatten() {
            for e in Walk::new(out).flat_map(Result::into_iter) {
                let is_dir = fs::canonicalize(e.path()).ok().is_some_and(|p| p.is_dir());
                if is_dir {
                    continue;
                }

                let is_elf = infer::get_from_path(e.path())
                    .ok()
                    .flatten()
                    .is_some_and(|ft| {
                        ft.mime_type() == "application/x-executable"
                            || ft.mime_type() == "application/x-sharedlib"
                    });

                if is_elf {
                    if let Ok(dep_tree) = DependencyAnalyzer::default().analyze(e.path()) {
                        shared_objects.extend(
                            dep_tree
                                .libraries
                                .into_values()
                                .flat_map(|l| fs::canonicalize(l.path).into_iter()),
                        );
                    }
                }
            }
        }
        // println!("{:?}", shared_objects);
        shared_objects
    }

    pub fn find_provided_shared_objects(&self) -> HashSet<PathBuf> {
        let mut shared_objects = HashSet::new();
        for out in self.build().iter().flatten() {
            for e in Walk::new(out).flat_map(Result::into_iter) {
                let is_dir = fs::canonicalize(e.path()).ok().is_some_and(|p| p.is_dir());
                if is_dir {
                    continue;
                }

                let is_so = infer::get_from_path(e.path())
                    .ok()
                    .flatten()
                    .is_some_and(|ft| {
                        ft.mime_type() == "application/x-executable"
                            || ft.mime_type() == "application/x-sharedlib"
                    });
                if is_so {
                    shared_objects.extend(fs::canonicalize(e.path()).into_iter());
                }
            }
        }
        // println!("provided so: {:?}", shared_objects);
        shared_objects
    }

    pub fn get_requisites(&self) -> Vec<Derivation> {
        // nix-store --query -R /nix/store/1y6qm7rx3pk5dzbr5r3hhzs1mk17sq7s-isl-0.20.drv

        Command::new("nix-store")
            .arg("--query")
            .arg("-R")
            .arg(&self.drv_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .unwrap()
            .stdout
            .lines()
            .flat_map(|l| l.into_iter())
            .filter(|l| l.ends_with(".drv"))
            .flat_map(|p| Derivation::read_drv(&p).into_iter())
            .collect()
    }

    pub fn find_used_c_headers(&self) -> HashSet<String> {
        let src_dir = if let Some(src_dir) = self.read_src_dir() {
            src_dir
        } else {
            return HashSet::new();
        };

        // find used headers
        let mut searcher = Searcher::new();
        searcher.set_binary_detection(BinaryDetection::none());
        // assumption: valid C/C++ code
        let header_include_regex_str = r##"^\s*#\s*include\s*(<|")([^>"]+)(>|").*$"##;
        let header_include_regex = RegexBuilder::new(header_include_regex_str)
            .multi_line(true)
            .build()
            .unwrap();
        let matcher = RegexMatcher::new(header_include_regex_str).unwrap();
        let mut used_headers: HashSet<String> = HashSet::new();
        for e in Walk::new(&src_dir).flat_map(Result::into_iter) {
            let is_dir = fs::canonicalize(e.path()).ok().is_some_and(|p| p.is_dir());

            if is_dir {
                continue;
            }
            searcher
                .search_path(
                    &matcher,
                    e.path(),
                    UTF8(|_, match_bytes| {
                        let include_path = header_include_regex
                            .captures(match_bytes)
                            .unwrap()
                            .get(2)
                            .unwrap()
                            .as_str();
                        let include_path = include_path
                            .rsplit_once('/')
                            .map(|s| s.1)
                            .unwrap_or(include_path);
                        used_headers.insert(include_path.to_string());
                        Ok(true) // continue reading the file
                    }),
                )
                .ok();
        }
        used_headers
    }

    pub fn build(&self) -> &Result<Vec<String>, std::io::Error> {
        self.build_outputs.get_or_init(|| {
            let build_path: &str = &self.drv_path;
            let build_path = if build_path.ends_with(".drv") {
                &format!("{}^*", build_path)
            } else {
                build_path
            };
            Command::new("nix")
                .arg("build")
                .arg(build_path)
                .arg("--no-link")
                .args(get_nix_flags())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .status()?;
            Ok(self.get_out_paths())
        })
    }

    pub fn get_provided_binaries(&self) -> HashSet<String> {
        self.build().as_ref().map_or_else(
            |_| HashSet::new(),
            |outputs| {
                let mut buf = HashSet::new();
                outputs.iter().for_each(|out| {
                    let mut out = PathBuf::from_str(out).unwrap();
                    out.push("bin");
                    if !out.exists() {
                        return;
                    }
                    Walk::new(out.as_path())
                        .flat_map(|r| r.into_iter())
                        .map(|p| p.file_name().to_string_lossy().into_owned())
                        .for_each(|f| {
                            buf.insert(f);
                        });
                });
                buf
            },
        )
    }

    pub fn get_provided_c_headers(&self) -> HashSet<String> {
        self.build().as_ref().map_or_else(
            |_| HashSet::new(),
            |outputs| {
                let mut buf = HashSet::new();
                outputs.iter().for_each(|out| {
                    let mut out = PathBuf::from_str(out).unwrap();
                    out.push("include");
                    if !out.exists() {
                        return;
                    }
                    Walk::new(out.as_path())
                        .flat_map(|r| r.into_iter())
                        .map(|p| p.file_name().to_string_lossy().into_owned())
                        .for_each(|f| {
                            buf.insert(f);
                        });
                });
                buf
            },
        )
    }
}

impl PartialEq for Derivation {
    fn eq(&self, other: &Self) -> bool {
        self.drv_path == other.drv_path
    }
}

impl Eq for Derivation {}

impl std::hash::Hash for Derivation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.drv_path.hash(state);
    }
}

// impl Hash for Derivation {}

fn try_extract_source_archive(src_archive_path: PathBuf) -> Option<TempDir> {
    let prefix = "nix-check-extract";
    let tmp_dir = tempfile::Builder::new().prefix(&prefix).tempdir().ok()?;

    if src_archive_path.to_str()?.ends_with(".tar.gz")
        || src_archive_path.to_str()?.ends_with(".tgz")
    {
        let tar_gz = File::open(src_archive_path).ok()?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_dir).ok()?;
        return Some(tmp_dir);
    } else if src_archive_path.to_str()?.ends_with(".tar.xz") {
        let tar_xz = File::open(src_archive_path).ok()?;
        let tar = XzDecoder::new(tar_xz);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_dir).ok()?;
        return Some(tmp_dir);
    } else if src_archive_path.to_str()?.ends_with(".tar.bz2") {
        let tar_bz2 = File::open(src_archive_path).ok()?;
        let tar = BzDecoder::new(tar_bz2);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_dir).ok()?;
        return Some(tmp_dir);
        // } else if src_archive_path.to_str()?.ends_with(".tar.lz") {
        //     let tar_lz = File::open(src_archive_path).ok()?;
        //     let tar = LzDecoder::new(tar_lz);
        //     let mut archive = Archive::new(tar);
        //     archive.unpack(&tmp_dir).ok()?;
        //     return Some(tmp_dir.into_path());
    }

    warn!(
        "unknown archive format for object: {}",
        src_archive_path.to_string_lossy()
    );
    None
}

pub fn eval_attr_to_drv_path(attr: &str) -> Option<String> {
    let output = Command::new("nix")
        .arg("eval")
        .arg(attr)
        .arg("--apply")
        .arg("attr: attr.drvPath")
        .arg("--json")
        .args(get_nix_flags())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    serde_json::from_reader(output.stdout?).ok()
}

pub fn get_store_hash(store_path: &str) -> String {
    store_path.strip_prefix("/nix/store/").unwrap_or(store_path)[..32].to_owned()
}
