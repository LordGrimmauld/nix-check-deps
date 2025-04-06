# nix-check-deps

This is a tool intended to locate unused buildInputs in nix packages. It is currently very much in development. Contributions are welcome.

## Example useage:

This project supplies a flake for easy usage.
`nix run github:lordgrimmauld/nix-check-deps` (or build or shell) will "just work".

### eval mode:
```
$ nix-check-deps nixpkgs#libvlc
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/q6pqipzzdnqgn5fp2vqk0b7didgd2g42-libXv-1.0.13.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/0vhigd2qb7zd0zjz3jzjx0knxdrzsm5y-libXvMC-1.0.14.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/z297g0vxva8hih0dfg3kxfhppgikxgpx-liboggz-1.1.3.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/360jp4p3ylizi27wf5v2l9xqkgb4bk9a-wayland-scanner-1.23.1.drv
```

### drv mode:
```
$ nix-check-deps /nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/z297g0vxva8hih0dfg3kxfhppgikxgpx-liboggz-1.1.3.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/q6pqipzzdnqgn5fp2vqk0b7didgd2g42-libXv-1.0.13.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/0vhigd2qb7zd0zjz3jzjx0knxdrzsm5y-libXvMC-1.0.14.drv
/nix/store/3w2ksdz7mnz9np2v46y9qp7yna4nqqqz-libvlc-3.0.21.drv has unused dependency: /nix/store/360jp4p3ylizi27wf5v2l9xqkgb4bk9a-wayland-scanner-1.23.1.drv
```

## Working principle

### eval mode:
- throws the input into `nix eval`
- reads the `.drv` path in nix store
- continues the drv mode code path

### drv mode:
- read the `.drv` file using `nix derivation show` and clap json parser
- read input derivations
- read `buildInputs`, `src` and some other relevant info from environment definition
- read `propagatedBuildInputs` from dependency `.drv` files
- collect all outputs of all dependencies used in `buildInputs`
- remove any inputs caused by propagation
- build the package
- scan the local copy of a package in /nix/store whether it contains references to its dependencies
- report unused

## reference scanning
This tool tries to be smart, but is currently not 100% accurate.
Various scanners are in use and enabled by default.

Output of unused dependencies is currently unordered.
Two runs of this tool might not generate the same ordering!

### imitating `nix why-depends --precise`
Like `nix why-depends --precise`, the simplest check is to see what store hashes are mentioned in the derivation outputs.
This scan will build the package and go through all files in all outputs matching for any of the drv hashes of inputs.
Note that any output hash of an input derivation will be accepted.
I have not yet found a suitable way to check which output of an input derivation is actually in use.

This check fails in multiple cases:
- vendoring (`cp` instead of symlink)
- `symlinkJoin`
- `override`/`overrideAttrs` (a fix is in progress)
- static linking
- expectations about runtime environment not explicitly expressed in nix
  (e.g. how `pycapnp` expects `capnp` to be in `$PYTHONPATH`)

### `--check-headers`
The `check-headers` feature will attempt to unpack the package source archive and scan for any `#include` directives.

Directives recognized:
- C-style `#include "..."`
- C++-style `#include <...>`
- general header extension `.h`
- C++-specifc headers `.hpp`
- Qt-headers `.qh`, `.qhpp`

Currently `.inl` is *not* recognized - inlines are weird. If there is a use to recognizing them, adding them is trivial.

All dependencies get built and their `include` path gets scanned for provided header files.
Any dependency that provides a header that is being included (intersection) will be marked as used.

### `--check-pyproject`
The `check-pyproject` feature will scan `pyproject.toml` by pep 508 rules
and accept any listed dependency (either optional or required) by nix `pname`.
This check requires extracting the source archive.

### `--check-shebangs`
The `check-shebangs` check will scan all files in the package source archive for the first line starting with `#!`.
Any programs used in a shebang directive will be collected.
Dependency packages providing any of the used programs in their `/bin` will be marked as used.

### `--check-shared-objects`
The `check-shared-objects` feature will scan all files provided by a package for their magic bytes defining mime type.
Any `x-application` or `x-sharedlib` will be opened.
If it successfully parses as ELF binary, the tree of shared objects in use will be extracted.
Dependency packages providing any of the used shared libraries will be marked as used.

Currently, all shared libraries will be scanned. In the future, this might be limited to scanning `/lib{,32,64,exec}`.

## Current limitations & future plans
Currently this tool can not differentiate between different outputs of dependencies.
To still give good results, it checks all outputs of dependencies and accepts a dependency if any of its outputs are used.
***This tool will report false positives***.

## Guidelines when contributing cleanup work to nixpkgs
Cleaning dependencies helps reduce the load on nixpkgs CI,
reduces flagged vulnerabilities by not depending on old unused stuff, and improves overall maintenance status of packages.

However, removing dependencies that seem unused **is potentially breaking**.
Any removal needs to be researched thoroughly. Following checklist might be helpful:

- [ ] find a potentially unused dependency with `nix-check-deps`
- [ ] upstream package has a commit/PR removing the dependency
- [ ] upstream package has a release that includes the commit/PR doing the remove
- [ ] upstream has a changelog documenting the removal of a dependency (or feature that was the sole last user of a dependency)
- [ ] upstream code does not refer to the dependency (code search tools might be useful here)
- [ ] why did nixpkgs add the dependency? This helps understanding impact of what might potentially break.
    - `git blame`
    - historical issues
    - historical PRs
    - old upstream build scripts referring to the dependency
- [ ] impact assessment
    - what depends on a package?
    - do all the tests still run?
    - [`nixpkgs-review`](https://github.com/Mic92/nixpkgs-review) can help, but is not a definitive answer. It just builds, it does not execute.
- [ ] open a pull request against nixpkgs. Be sure to also follow guidelines on [CONTRIBUTING.md](https://github.com/NixOS/nixpkgs/blob/master/CONTRIBUTING.md)
- [ ] add the `closure size` tag to your pull request
- [ ] explain why it is safe to drop the dependency, explain the testing and research done.
