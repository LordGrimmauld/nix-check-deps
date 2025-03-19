mod args;
use crate::args::Cli;
use clap::Parser;
use ignore::Walk;

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::Bytes, BinaryDetection, Searcher},
};

use std::{
    fs::{self},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

fn get_store_hash(store_path: &str) -> String {
    return store_path
        .strip_prefix("/nix/store/")
        .unwrap_or(&store_path)[..32]
        .to_owned();
}

fn main() {
    let cli = Cli::parse();
    let attr: String = cli.attr.to_string();

    // make sure the package exists, so it can be scanned
    let pkg_outputs_raw = Command::new("nix")
        .arg("build")
        .arg(&attr)
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

    // get listed dependencies
    let output = Command::new("nix")
        .arg("eval")
        .arg(&attr)
        .arg("--apply")
        // .arg("p: let passthru = builtins.concatMap (dep: builtins.concatMap (dep: dep.all) dep.propagatedBuildInputs) p.buildInputs; in map (dep: [ \"${dep.out or dep}\" ]) (builtins.filter (dep: !(builtins.elem dep passthru)) p.buildInputs)") // TODO: figure out how to only scan the specific dep
        .arg("p: let deps = builtins.map (dep: dep.all) p.buildInputs; passthru = builtins.concatMap (dep: builtins.concatMap (dep: dep.all) dep.propagatedBuildInputs) p.buildInputs; in builtins.map (builtins.filter (dep: ! (builtins.elem dep passthru) )) deps")
        .arg("--json")
        // .arg("--impure")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    let mut deps: Vec<Vec<String>> = serde_json::from_reader(output.stdout.unwrap()).unwrap();

    let mut searcher = Searcher::new();
    searcher.set_binary_detection(BinaryDetection::none());
    for result in Walk::new(&pkg_outputs[0]) {
        let e = result.unwrap();
        let is_file = e.file_type().map_or(false, |f| f.is_file());
        let is_link = e.file_type().map_or(false, |f| f.is_symlink());

        if is_file {
            deps.retain(|dep_drv| {
                let mut found = false;
                for dep in dep_drv {
                    let matcher = RegexMatcher::new(&get_store_hash(dep)).unwrap();
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
                    if found {
                        break;
                    }
                }
                return !found;
            });
        } else if is_link {
            deps.retain(|dep_drv| {
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

    for dep_drv in deps {
        if let Some(dep) = dep_drv.get(0) {
            println!("{} has unused dependency: {}", attr, dep);
        }
    }
}
