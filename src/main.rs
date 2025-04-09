mod args;
mod derivation;
use crate::args::Cli;
use clap::Parser;
use derivation::Derivation;
use ignore::Walk;
use log::info;
use once_cell::sync::OnceCell;
use rayon::ThreadPoolBuilder;
// use nix_compat::derivation::Derivation;
use regex::Regex;
use serde_json::json;
use std::{collections::HashMap, path::Path, time::Instant};

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::Bytes, BinaryDetection, Searcher},
};

use std::fs::{self};

static NIX_FLAGS: OnceCell<Vec<String>> = OnceCell::new();

pub fn get_nix_flags() -> &'static Vec<String> {
    NIX_FLAGS.get_or_init(Vec::new)
}

fn main() {
    env_logger::init();
    let permitted_unused_deps = vec![
        Regex::new("iconv-").unwrap(),
        Regex::new("gtest-").unwrap(),
        Regex::new("gbenchmark-").unwrap(),
        Regex::new("wayland-protocols").unwrap(),
        Regex::new("-dbus").unwrap(),
        Regex::new("-polkit").unwrap(),
        Regex::new("-systemd").unwrap(),
        Regex::new("python3\\..*-mock-").unwrap(),
        Regex::new("python3\\..*-pytest-").unwrap(),
        Regex::new("perl-?5\\.").unwrap(),
        Regex::new("-check-").unwrap(),
        Regex::new(r"-hook(\.drv(\^\**)?)?$").unwrap(),
    ];

    let cli = Cli::parse();
    let attr: String = cli.attr.to_string();
    NIX_FLAGS.set(cli.nix_flags.unwrap_or_else(Vec::new)).ok();

    let drv_logic = attr.ends_with(".drv") && Path::new(&attr).exists();
    let attr = if drv_logic {
        attr
    } else {
        derivation::eval_attr_to_drv_path(&attr).unwrap()
    };

    let drv = Derivation::read_drv(&attr).unwrap();

    // [ ( dependent, [ dependency ] ) ]
    let mut scan_roots: Vec<(Derivation, Vec<Derivation>)> = Vec::new();

    if cli.tree {
        for drv in drv.get_requisites() {
            let deps = drv.read_deps();
            scan_roots.push((drv, deps));
        }
    }

    let deps = drv.read_deps();
    scan_roots.insert(0, (drv, deps)); // insert top-level at the start so the nix build can build all dependents at once

    let pool = ThreadPoolBuilder::new()
        .num_threads(cli.jobs)
        .build()
        .unwrap();

    let skipped: Vec<String> = cli.skip.split(",").map(str::to_owned).collect();

    let mut found_unused: HashMap<String, Vec<String>> = HashMap::new();

    // FIXME: this doesn't really check in parallel, this never worked in the first place
    pool.install(|| {
        scan_roots.iter_mut().for_each(|(root, dep_relations)| {
            if skipped.iter().any(|s| root.matches_pname(s)) {
                return;
            }

            // debug!("rels {:?}", dep_relations);
            // debug!("root {:?}", root.drv_path);

            dep_relations.retain(|dep_drv| {
                !permitted_unused_deps
                    .iter()
                    .any(|re| re.is_match(&dep_drv.drv_path))
            });

            if cli.check_headers || cli.list_used_headers {
                let start = Instant::now();
                let used_headers = root.find_used_c_headers();
                dep_relations.retain(|dep_drv| {
                    dep_drv.build().as_ref().unwrap();
                    !derivation::test_headers_of_package_used(
                        &used_headers,
                        &mut dep_drv.get_out_paths(),
                    )
                });
                if cli.list_used_headers {
                    for header in used_headers {
                        info!("{} uses header: {}", root.drv_path, header);
                    }
                }
                info!("check-headers took {:.2?} seconds", start.elapsed());
            }

            if cli.skip_dep_usage_check {
                return;
            }

            if cli.check_pyproject {
                let start = Instant::now();
                let used_py_deps = root.find_used_pyproject_deps();
                dep_relations
                    .retain(|dep_drv| !used_py_deps.iter().any(|py| dep_drv.matches_pname(py)));
                info!("check-pyproject took {:.2?} seconds", start.elapsed());
            }

            if cli.check_shebangs {
                let start = Instant::now();
                let used_shebangs = root.find_used_shebangs();
                dep_relations.retain(|dep_drv| {
                    !dep_drv
                        .get_provided_binaries()
                        .intersection(&used_shebangs)
                        .any(|_| true)
                });
                info!("check-shebangs took {:.2?} seconds", start.elapsed());
            }

            if cli.check_shared_objects {
                let start = Instant::now();
                let used_shared_objects = root.find_used_shared_objects();
                dep_relations.retain(|dep_drv| {
                    !dep_drv
                        .find_provided_shared_objects()
                        .intersection(&used_shared_objects)
                        .any(|_| true)
                });
                info!("check-shared-objects took {:.2?} seconds", start.elapsed());
            }

            // make sure the package exists in local store so it can be scanned
            let pkg_outputs = if let Ok(pkg_outputs) = root.build() {
                pkg_outputs
            } else {
                log::error!(
                    "derivation {} does not build, skipping checks...",
                    root.drv_path
                );
                return;
            };

            let mut searcher = Searcher::new();
            searcher.set_binary_detection(BinaryDetection::none());
            for output in pkg_outputs {
                for e in Walk::new(output).flat_map(Result::into_iter) {
                    let is_file = e.file_type().is_some_and(|f| f.is_file());
                    let is_link = e.file_type().is_some_and(|f| f.is_symlink());

                    if is_file {
                        dep_relations.retain(|dep_drv| {
                            let mut found = false;
                            let regex: String = dep_drv
                                .get_out_paths()
                                .iter()
                                .map(|dep| derivation::get_store_hash(dep))
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
                                .ok();
                            !found
                        });
                    } else if is_link {
                        dep_relations.retain(|dep_drv| {
                            let p = fs::read_link(e.path()).unwrap();
                            for dep in dep_drv.get_out_paths() {
                                if p.to_string_lossy()
                                    .contains(&derivation::get_store_hash(&dep))
                                {
                                    return false;
                                }
                            }
                            true
                        });
                    }
                }
            }

            let mut found_unused_drv = Vec::new();
            for dep in dep_relations.iter() {
                found_unused_drv.push(dep.drv_path.clone());
                // fixme: json
            }
            if !found_unused_drv.is_empty() {
                found_unused.insert(root.drv_path.clone(), found_unused_drv);
            }
        });
    });

    if cli.json {
        println!("{}", json!(found_unused));
    } else {
        for (root, unused) in found_unused {
            for dep in unused {
                println!("{} has unused dependency: {}", root, dep);
            }
        }
    }
}
