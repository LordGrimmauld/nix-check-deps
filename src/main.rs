mod args;
mod derivation;
use crate::args::Cli;
use clap::Parser;
use derivation::Derivation;
use ignore::Walk;
use rayon::ThreadPoolBuilder;
// use nix_compat::derivation::Derivation;
use regex::Regex;
use std::{collections::HashSet, path::Path};

use grep::{
    regex::RegexMatcher,
    searcher::{sinks::Bytes, BinaryDetection, Searcher},
};

use std::fs::{self};

fn main() {
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

    let drv_logic = attr.ends_with(".drv") && Path::new(&attr).exists();
    let attr = if drv_logic {
        attr
    } else {
        derivation::eval_attr_to_drv_path(&attr).unwrap()
    };

    let drv = Derivation::read_drv(&attr).unwrap();

    // (dependent, {dependency_name -> [outputs] } )
    let mut scan_roots: Vec<(Derivation, HashSet<Derivation>)> = if !cli.reverse {
        let deps = drv.read_deps();
        vec![(drv, deps)]
    } else {
        drv.referrers()
            .iter()
            .map(|r| {
                (
                    Derivation::read_drv(r).unwrap(),
                    [drv.clone()]
                        .iter()
                        .cloned()
                        .collect::<HashSet<Derivation>>(),
                )
            })
            .collect()
    };

    let pool = ThreadPoolBuilder::new()
        .num_threads(cli.jobs)
        .build()
        .unwrap();

    let skipped: Vec<String> = cli.skip.split(",").map(str::to_owned).collect();

    pool.install(|| {
        scan_roots.iter_mut().for_each(|(root, dep_relations)| {
            if skipped.iter().any(|s| root.matches_pname(s)) {
                return;
            }

            // println!("rels {:?}", dep_relations);
            // println!("root {:?}", root.drv_path);

            dep_relations.retain(|dep_drv| {
                !permitted_unused_deps
                    .iter()
                    .any(|re| re.is_match(&dep_drv.drv_path))
            });

            if cli.check_headers || cli.list_used_headers {
                let used_headers = root.find_used_c_headers();
                dep_relations.retain(|dep_drv| {
                    derivation::build_drv(&dep_drv.drv_path).unwrap();
                    !derivation::test_headers_of_package_used(
                        &used_headers,
                        &mut dep_drv.get_out_paths(),
                    )
                });
                if cli.list_used_headers {
                    for header in used_headers {
                        println!("{} uses header: {}", root.drv_path, header);
                    }
                }
            }

            if cli.skip_dep_usage_check {
                return;
            }

            // make sure the package exists in local store so it can be scanned
            let pkg_outputs = if let Some(pkg_outputs) = derivation::build_drv(&root.drv_path) {
                pkg_outputs
            } else {
                println!(
                    "derivation {} does not build, skipping checks...",
                    root.drv_path
                );
                return;
            };

            let mut searcher = Searcher::new();
            searcher.set_binary_detection(BinaryDetection::none());
            for output in pkg_outputs {
                for result in Walk::new(&output) {
                    let e = result.unwrap();
                    let is_file = e.file_type().map_or(false, |f| f.is_file());
                    let is_link = e.file_type().map_or(false, |f| f.is_symlink());

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
                                .unwrap();
                            return !found;
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

            for dep in dep_relations.iter() {
                println!("{} has unused dependency: {}", root.drv_path, dep.drv_path);
            }
        });
    });
}
