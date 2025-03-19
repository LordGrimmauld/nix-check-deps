# takes a package, returns its build inputs with all their outputs, removing propagatedBuildInputs
p:
let
  deps = builtins.map (dep: dep.all) p.buildInputs;
  passthru = builtins.concatMap (
    dep: builtins.concatMap (dep: dep.all) dep.propagatedBuildInputs
  ) p.buildInputs;
in
builtins.map (builtins.filter (dep: !(builtins.elem dep passthru))) deps
