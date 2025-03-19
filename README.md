# nix-check-deps

This is a tool intended to locate unused buildInputs in nix packages. It is currently very much in development. Contributions are welcome.

## Example useage:
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

## Functioning
This tool throws the input into `nix eval` and reads `buildInputs`. It then removes any `propagatedBuildInputs`, as those tend to be unused (which is not a bad thing for those specifically).
It downloads/builds the package being checked and then scans the local copy in nix store whether it contain the hash part of a derivation output referencing the dependencies.

This can be validated using `nix why-depends --precise`.

## Current limitations & future plans
Currently this tool can not differentiate between different outputs of dependencies. To still give good results, it checks all outputs of dependencies and accepts a dependency if any of its outputs are used.
This tool also relies on flake tooling and accessing the actual nix attrs, which makes using it in CI a little bothersome.
Fixing these two limitations is certainly a goal, and would allow this tool to even one day be used in nixpkgs CI, testing package changes.
