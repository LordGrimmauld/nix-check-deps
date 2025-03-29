mod args;
use crate::args::Cli;
use clap::Parser;
use flate2::read::GzDecoder;
use ignore::Walk;
use nix_compat::derivation::Derivation;
use regex::Regex;
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

fn read_src_from_drv(drv_path: &str) -> Option<PathBuf> {
    let drv = Derivation::from_aterm_bytes(&fs::read(drv_path).ok()?).ok()?;
    let src_drv = drv.environment.get("src")?.to_string();
    // TODO: maybe integrate with https://github.com/milahu/nix-build-debug or similar

    let build_results = build_drv(&src_drv)?;
    let src_archive_path = PathBuf::from(build_results.get(0)?);
    if !src_archive_path.exists() {
        return None;
    }
    if src_archive_path.is_dir() {
        return Some(src_archive_path);
    }

    println!("creating tmpdir to unpack...");

    let tmp_dir = tempfile::Builder::new()
        .prefix(&format!("nix-check-extract-{}", get_store_hash(drv_path)))
        .tempdir()
        .ok()?;

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
    // unknown archive format
    None
}

fn read_deps_from_drv(drv_path: &str) -> HashMap<String, Vec<String>> {
    let drv = Derivation::from_aterm_bytes(&fs::read(drv_path).unwrap()).unwrap();

    let dev_inputs: Vec<String> = drv
        .environment
        .get("buildInputs")
        .map_or_else(Vec::new, |s| {
            s.to_string()
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        });

    let mut dep_relations: HashMap<String, Vec<String>> = HashMap::new();
    let mut propagated: Vec<String> = Vec::new();
    let check_inputs = drv
        .environment
        .get("checkInputs")
        .map_or_else(Vec::new, |s| {
            s.to_string()
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        });

    let all_inputs = drv.input_derivations.keys();
    for input in all_inputs {
        let dep_drv_path = input.to_absolute_path();
        let dep_drv = Derivation::from_aterm_bytes(&fs::read(&dep_drv_path).unwrap()).unwrap();
        let mut propagated_drvs = dep_drv
            .environment
            .get("propagatedBuildInputs")
            .map_or_else(Vec::new, |s| {
                s.to_string()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect()
            });
        propagated.append(&mut propagated_drvs);
        let outputs: Vec<String> = dep_drv
            .outputs
            .values()
            .map(|o| o.path.as_ref().unwrap().to_absolute_path())
            .collect();

        // println!("drv input: {:?}", dep_drv_path);
        // println!("outputs: {:?}", outputs);
        // println!("propagated: {:?}", propagated_drvs);

        if outputs.iter().any(|o| dev_inputs.contains(o)) {
            dep_relations.insert(dep_drv_path, outputs);
        }
    }

    dep_relations.retain(|_, v| !propagated.iter().any(|p| v.contains(p)));
    dep_relations.retain(|_, v| !check_inputs.iter().any(|p| v.contains(p)));
    // println!("dev inputs: {:?}", dev_inputs);
    // println!("check inputs: {:?}", check_inputs);
    // println!("dep relations: {:?}", dep_relations);
    return dep_relations;
}

fn read_deps_from_attr(attr: &str) -> HashMap<String, Vec<String>> {
    // get listed dependencies
    let output = Command::new("nix")
        .arg("eval")
        .arg(&attr)
        .arg("--apply")
        // .arg("p: let passthru = builtins.concatMap (dep: builtins.concatMap (dep: dep.all) dep.propagatedBuildInputs) p.buildInputs; in map (dep: [ \"${dep.out or dep}\" ]) (builtins.filter (dep: !(builtins.elem dep passthru)) p.buildInputs)") // TODO: figure out how to only scan the specific dep
        .arg("p: let deps = builtins.map (dep: dep.all) p.buildInputs; passthru = builtins.concatMap (dep: builtins.concatMap (dep: dep.all) dep.propagatedBuildInputs) p.buildInputs; in builtins.map (builtins.filter (dep: ! (builtins.elem dep passthru || builtins.elem dep p.checkInputs or []) )) deps")
        .arg("--json")
        // .arg("--impure")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    let deps: Vec<Vec<String>> = serde_json::from_reader(output.stdout.unwrap()).unwrap();
    let mut dep_relations: HashMap<String, Vec<String>> = HashMap::new();
    for dep in deps {
        if let Some(example) = dep.get(0) {
            dep_relations.insert(example.clone(), dep);
        }
    }
    return dep_relations;
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
    let build_path = if drv_logic {
        format!("{}^*", &attr)
    } else {
        attr.clone()
    };

    // make sure the package exists, so it can be scanned
    let pkg_outputs = build_drv(&build_path).unwrap();

    let mut dep_relations = if drv_logic {
        read_deps_from_drv(&attr)
    } else {
        read_deps_from_attr(&attr)
    };

    dep_relations.retain(|_, v| {
        !v.iter()
            .any(|dep| permitted_unused_deps.iter().any(|re| re.is_match(dep)))
    });

    if drv_logic && cli.check_headers {
        if let Some(src_dir) = read_src_from_drv(&attr) {
            println!("{:?}", src_dir);

            // find used headers
            let mut searcher = Searcher::new();
            searcher.set_binary_detection(BinaryDetection::none());
            let header_include_regex_str =
                r##"^#include (<|")(.*\/)*(.*\.q?h(pp?))(>|") *((\/\/.*)|(\/*))?\n?$"##;
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
                            // println!("{}", include_path);
                            Ok(true) // stop reading the file
                        }),
                    )
                    .unwrap();
            }

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
                            println!("found matching header {} for pkg {}", header, dep_output);
                            return false;
                        }
                        // if used_headers
                        //     .iter()
                        //     .any(|header| e.path().to_string_lossy().ends_with(header))
                        // {
                        //     return false;
                        // }
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
