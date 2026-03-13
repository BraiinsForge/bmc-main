# autopatchelf-binaries: Patch ELF binaries using autoPatchelfHook + runtimeDependencies.
#
# Applied via overrideAttrs on the buildCrate derivation.
# autoPatchelfHook runs in postFixupHooks (after --shrink-rpath),
# so the rpaths it sets are not stripped.
{ lib, autoPatchelfHook }:
{ drv, runtimeDeps ? [ ] }:
drv.overrideAttrs (prev: {
  nativeBuildInputs = (prev.nativeBuildInputs or [ ]) ++ [ autoPatchelfHook ];
  # runtimeDependencies: autoPatchelfHook appends these /lib paths
  # to rpath of all dynamic executables, regardless of DT_NEEDED.
  runtimeDependencies = (prev.runtimeDependencies or [ ]) ++ (map lib.getLib runtimeDeps);
})
