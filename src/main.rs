mod args;
use crate::args::Cli;
use clap::Parser;
use ignore::Walk;
use nix_compat::derivation::Derivation;
use regex::Regex;
use std::path::Path;

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::Bytes, BinaryDetection, Searcher},
};

use std::{
    collections::HashMap,
    fs::{self},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

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
    let pkg_outputs_raw = Command::new("nix")
        .arg("build")
        .arg(build_path)
        .arg("--print-out-paths")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
        .stdout
        .unwrap();

    let pkg_outputs: Vec<String> = BufReader::new(pkg_outputs_raw)
        .lines()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut dep_relations = if drv_logic {
        read_deps_from_drv(&attr)
    } else {
        read_deps_from_attr(&attr)
    };

    dep_relations.retain(|_, v| {
        !v.iter()
            .any(|dep| permitted_unused_deps.iter().any(|re| re.is_match(dep)))
    });

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
