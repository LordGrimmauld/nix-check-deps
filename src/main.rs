mod args;
use crate::args::Cli;
use clap::Parser;
use flate2::read::GzDecoder;
use ignore::Walk;
// use nix_compat::derivation::Derivation;
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::File,
    path::{Path, PathBuf},
};
use tar::Archive;
use xz::read::XzDecoder;

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::Bytes, sinks::UTF8, BinaryDetection, Searcher},
};

use std::{
    collections::HashMap,
    fs::{self},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[derive(Deserialize, Hash, Eq, PartialEq, Debug, Clone)]
struct DrvOutput {
    path: String,
}

impl DrvOutput {
    fn path(self) -> String {
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
    propagated_build_inputs: Option<String>,
    src: Option<String>,
}

impl DrvEnv {
    fn get_build_inputs(&self) -> Vec<String> {
        self.build_inputs.as_ref().map_or_else(Vec::new, |s| {
            s.split_whitespace().map(str::to_owned).collect()
        })
    }

    #[allow(dead_code)]
    fn get_check_inputs(&self) -> Vec<String> {
        self.build_inputs.as_ref().map_or_else(Vec::new, |s| {
            s.split_whitespace().map(str::to_owned).collect()
        })
    }
    fn get_propagated_build_inputs(&self) -> Vec<String> {
        self.build_inputs.as_ref().map_or_else(Vec::new, |s| {
            s.split_whitespace().map(str::to_owned).collect()
        })
    }
}

#[derive(Deserialize, Eq, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Derivation {
    env: DrvEnv,
    outputs: HashMap<String, DrvOutput>,
    input_drvs: HashMap<String, DrvInput>,
    #[serde(skip_deserializing)]
    drv_path: String,
}

impl Derivation {
    fn read_drv(drv_path: &str) -> Option<Self> {
        let drv_path = if drv_path.ends_with(".drv") {
            &format!("{}^*", drv_path)
        } else {
            drv_path
        };
        let output = Command::new("nix")
            .arg("derivation")
            .arg("show")
            .arg(&drv_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .ok()?;
        let drvs: HashMap<String, Derivation> = serde_json::from_reader(output.stdout?).unwrap(); // .ok()?; // FIXME ?
        drvs.into_iter().last().map(|(path, mut drv)| {
            drv.drv_path = path.to_owned();
            drv
        })
    }

    fn get_input_drv_paths(&self) -> Vec<String> {
        self.input_drvs.clone().into_keys().collect()
    }

    fn read_src_dir(&self) -> Option<PathBuf> {
        let src_drv = self.env.src.as_ref()?;
        // TODO: maybe integrate with https://github.com/milahu/nix-build-debug or similar

        let build_results = build_drv(&src_drv)?;
        let src_archive_path = PathBuf::from(build_results.get(0)?);
        if !src_archive_path.exists() {
            return None;
        }
        if src_archive_path.is_dir() {
            return Some(src_archive_path);
        }

        try_extract_source_archive(src_archive_path)
    }

    fn read_deps(&self) -> HashMap<String, Vec<String>> {
        let dev_inputs: Vec<String> = self.env.get_build_inputs();

        let mut dep_relations: HashMap<String, Vec<String>> = HashMap::new();
        let mut propagated: Vec<String> = Vec::new();
        let check_inputs = self.env.check_inputs.as_ref();

        let all_inputs = self.get_input_drv_paths();
        for dep_drv_path in all_inputs {
            let dep_drv = Derivation::read_drv(&dep_drv_path).unwrap();
            let propagated_drvs = dep_drv.env.get_propagated_build_inputs();
            let outputs: Vec<String> = dep_drv.outputs.into_values().map(DrvOutput::path).collect();

            if outputs.iter().any(|o| dev_inputs.contains(o)) {
                dep_relations.insert(dep_drv_path, outputs);
            }
            propagated.append(&mut propagated_drvs.clone());

            // println!("drv input: {:?}", dep_drv_path);
            // println!("outputs: {:?}", outputs);
            // println!("propagated: {:?}", propagated_drvs);
        }

        dep_relations.retain(|_, v| !propagated.iter().any(|p| v.contains(p)));
        dep_relations.retain(|_, v| !check_inputs.iter().any(|p| v.contains(p)));
        // println!("dev inputs: {:?}", dev_inputs);
        // println!("check inputs: {:?}", check_inputs);
        // println!("dep relations: {:?}", dep_relations);
        return dep_relations;
    }
}

fn try_extract_source_archive(src_archive_path: PathBuf) -> Option<PathBuf> {
    let prefix = "nix-check-extract";
    let tmp_dir = tempfile::Builder::new().prefix(&prefix).tempdir().ok()?;

    if src_archive_path.to_str()?.ends_with(".tar.gz") {
        let tar_gz = File::open(src_archive_path).ok()?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_dir).ok()?;
        return Some(tmp_dir.into_path());
    } else if src_archive_path.to_str()?.ends_with(".tar.xz") {
        let tar_xz = File::open(src_archive_path).ok()?;
        let tar = XzDecoder::new(tar_xz);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_dir).ok()?;
        return Some(tmp_dir.into_path());
    }

    println!("unknown archive format");
    None
}

fn eval_attr_to_drv_path(attr: &str) -> Option<String> {
    let output = Command::new("nix")
        .arg("eval")
        .arg(&attr)
        .arg("--apply")
        .arg("attr: attr.drvPath")
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    serde_json::from_reader(output.stdout?).ok()
}

fn get_store_hash(store_path: &str) -> String {
    return store_path
        .strip_prefix("/nix/store/")
        .unwrap_or(&store_path)[..32]
        .to_owned();
}

fn main() {
    let permitted_unused_deps = vec![
        Regex::new("iconv-").unwrap(),
        Regex::new("gtest-").unwrap(),
        Regex::new("gbenchmark-").unwrap(),
        Regex::new("-check-").unwrap(),
        Regex::new("pytest-check-hook").unwrap(),
        Regex::new("python3\\..*-mock-").unwrap(),
        Regex::new("python3\\..*-pytest-").unwrap(),
        Regex::new("perl-?5\\.").unwrap(),
        Regex::new("unittest-check-hook").unwrap(),
    ];

    let cli = Cli::parse();
    let attr: String = cli.attr.to_string();

    let drv_logic = attr.ends_with(".drv") && Path::new(&attr).exists();
    let attr = if drv_logic {
        attr
    } else {
        eval_attr_to_drv_path(&attr).unwrap()
    };

    // make sure the package exists in local store so it can be scanned
    let pkg_outputs = build_drv(&attr).unwrap();

    let drv = Derivation::read_drv(&attr).unwrap();

    let mut dep_relations = drv.read_deps();
    // println!("{:?}", dep_relations);

    dep_relations.retain(|_, v| {
        !v.iter()
            .any(|dep| permitted_unused_deps.iter().any(|re| re.is_match(dep)))
    });

    if cli.check_headers {
        if let Some(src_dir) = drv.read_src_dir() {
            let used_headers = find_used_c_headers(src_dir);
            dep_relations.retain(|dep, dep_outputs| {
                build_drv(dep).unwrap();
                for dep_output in dep_outputs {
                    for result in Walk::new(&dep_output) {
                        let e = result.unwrap();
                        let is_file = e.file_type().map_or(false, |f| f.is_file());
                        if !is_file {
                            continue;
                        }
                        let header = e
                            .into_path()
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string();
                        if used_headers.contains(&header) {
                            // println!("found matching header {} for pkg {}", header, dep_output);
                            return false;
                        }
                    }
                }
                return true;
            });
        }
    }

    let mut searcher = Searcher::new();
    searcher.set_binary_detection(BinaryDetection::none());
    for output in pkg_outputs {
        for result in Walk::new(&output) {
            let e = result.unwrap();
            let is_file = e.file_type().map_or(false, |f| f.is_file());
            let is_link = e.file_type().map_or(false, |f| f.is_symlink());

            if is_file {
                dep_relations.retain(|_, dep_drv| {
                    let mut found = false;
                    let regex: String = dep_drv
                        .iter()
                        .map(|dep| get_store_hash(dep))
                        .collect::<Vec<String>>()
                        .join("|");
                    let matcher = RegexMatcher::new(&regex).unwrap();
                    searcher
                        .search_path(
                            &matcher,
                            e.path(),
                            Bytes(|_, _| {
                                found = true;
                                Ok(false) // stop reading the file
                            }),
                        )
                        .unwrap();
                    return !found;
                });
            } else if is_link {
                dep_relations.retain(|_, dep_drv| {
                    let p = fs::read_link(e.path()).unwrap();
                    for dep in dep_drv {
                        if p.to_string_lossy().contains(&get_store_hash(dep)) {
                            return false;
                        }
                    }
                    true
                });
            }
        }
    }

    for dep in dep_relations.keys() {
        println!("{} has unused dependency: {}", attr, dep);
    }
}

fn find_used_c_headers(src_dir: PathBuf) -> HashSet<String> {
    // find used headers
    let mut searcher = Searcher::new();
    searcher.set_binary_detection(BinaryDetection::none());
    let header_include_regex_str =
        r##"^#include (<|")(.*\/)*(.*\.q?h(pp)?)(>|") *((\/\/.*)|(\/*))?\n?$"##;
    let header_include_regex = Regex::new(header_include_regex_str).unwrap();
    let matcher = RegexMatcher::new(header_include_regex_str).unwrap();
    let mut used_headers: HashSet<String> = HashSet::new();
    for result in Walk::new(&src_dir) {
        let e = result.unwrap();
        let is_file = e.file_type().map_or(false, |f| f.is_file());
        if !is_file {
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
                        .get(3)
                        .unwrap()
                        .as_str();
                    used_headers.insert(include_path.to_string());
                    Ok(true) // stop reading the file
                }),
            )
            .unwrap();
    }
    used_headers
}

fn build_drv(build_path: &str) -> Option<Vec<String>> {
    let build_path = if build_path.ends_with(".drv") {
        &format!("{}^*", build_path)
    } else {
        build_path
    };
    let pkg_outputs_raw = Command::new("nix")
        .arg("build")
        .arg(build_path)
        .arg("--print-out-paths")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?
        .stdout?;

    Some(
        BufReader::new(pkg_outputs_raw)
            .lines()
            .collect::<Result<_, _>>()
            .unwrap_or_else(|_| Vec::new()),
    )
}
