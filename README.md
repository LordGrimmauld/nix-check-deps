# nix-check-deps

This is a tool intended to locate unused buildInputs in nix packages. It is currently very much in development. Contributions are welcome.

## Example useage:
### eval mode:
```
$ target/release/nix-check-deps nixpkgs#libvlc

nixpkgs#libvlc has unused dependency: /nix/store/jn71ik0fcnns74xwlfmp64kz6q78l6ix-libXpm-3.5.17-bin
nixpkgs#libvlc has unused dependency: /nix/store/5ha4yyg7j1p92ya6d8kn6cy2skw8zhkq-libXv-1.0.13
nixpkgs#libvlc has unused dependency: /nix/store/372hs7bm3d5hc2skp232g0mayj06yjl2-libXvMC-1.0.14
nixpkgs#libvlc has unused dependency: /nix/store/kw55cxff3ryv9b0ml36xhxybjrs0fzs3-liboggz-1.1.2
nixpkgs#libvlc has unused dependency: /nix/store/10slx6l4inhzlhc6jhgzxzah75s9y3xz-libraw1394-2.1.2
nixpkgs#libvlc has unused dependency: /nix/store/6asmmwf1xsw18jnifpj4gh8dds8vdfy9-libspatialaudio-0.3.0
nixpkgs#libvlc has unused dependency: /nix/store/b8mpkzizrf8mb7jpi70ja0dg4384mhci-v4l-utils-1.24.1
nixpkgs#libvlc has unused dependency: /nix/store/ahwddnywa65240wvzw388d426x73crrq-libvdpau-1.5
nixpkgs#libvlc has unused dependency: /nix/store/57rayvb28n66prgyavcbz2kk1qv3zydw-systemd-257.3
nixpkgs#libvlc has unused dependency: /nix/store/1kbhvxdw3p9z7ivnikdm2rig1zprpg4i-wayland-scanner-1.23.1
nixpkgs#libvlc has unused dependency: /nix/store/bwrphid2brw59bkhjkmpz8w2hqgxqszp-wayland-protocols-1.41
```

### drv mode:
```
$ nix-check-deps /nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv

/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/64gkszpswcwhqmdw766kxza7s7fizbqb-wayland-protocols-1.41.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/19kdaydrmspb678fxsn5sgqyq9kzgzix-libspatialaudio-0.3.0.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/0a03akzhpvnfflkkj31gaz4hdvl5gcjw-libraw1394-2.1.2.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/ksjdx8bbqirvbx71ns98nf7rz2ci13bp-libXpm-3.5.17.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/hprxqm88xi1gixczcjb3lmxp6ccmirya-systemd-257.3.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/1paj3glc6lryhqzrx1q7w7qbgb6nbkkk-liboggz-1.1.2.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/mmc4qds7ss2jjd3zx3b6lf1j8fg1k46k-libXvMC-1.0.14.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/9b6v60ssyr23m6ibj2l7pld14xw71k2g-wayland-scanner-1.23.1.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/a5k68081l5l4nm1gyc4cs91hx4ify4fz-libXv-1.0.13.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/rakglb4568wprwx8zij1l65pkd0b2sgf-libvdpau-1.5.drv
/nix/store/s1zd8bhnss7cvpz504pg9d4py168g46k-libvlc-3.0.21.drv has unused dependency: /nix/store/nq654n28y0yvg6vph37jcn8igd20vx4r-v4l-utils-1.24.1.drv
```

## Working principle

### eval mode:
- throws the input into `nix eval`
- reads `buildInputs`
- remove any `propagatedBuildInputs` propagated by dependencies, as those tend to be unused (this is generally acceptable)
- build the package
- scan the local copy in /nix/store whether it contains the hash part of a derivation output referencing the dependencies
- report unused

### drv mode:
- read the `.drv` file using the tvix drv parser
- read input derivations
- read `buildInputs` from environment definition
- read `propagatedBuildInputs` from dependency `.drv` files
- collect all outputs of all dependencies used in `buildInputs`
- remove any inputs caused by propagation
- build the package
- scan the local copy in /nix/store whether it contains the hash part of a derivation output referencing the dependencies
- report unused

Output of unused dependencies is unordered.
Output can be validated using `nix why-depends --precise`.

## Current limitations & future plans
Currently this tool can not differentiate between different outputs of dependencies. To still give good results, it checks all outputs of dependencies and accepts a dependency if any of its outputs are used.
As of recent changes, this tool can sue both flake attrs as well as a path to a `/nix/store/*.drv`file, enabling use together with e.g. `nix-eval-jobs` to scan all of nixpkgs.
This tool will report false positives if outputs are either statically linked or resources are copied instead of symlinked.
