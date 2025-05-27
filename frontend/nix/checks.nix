{ pkgs, src, yarnFiles }:

let
  run =
    { name
    , script
    , postScript ? ""
    ,
    }: pkgs.stdenv.mkDerivation {
      name = "bmc-fe-check-${name}";
      inherit src;
      buildInputs = [ pkgs.yarn ];
      buildPhase = ''
        export HOME=$(pwd)
        cp -r ${yarnFiles}/. -t .

        mkdir -p $out

        ${script} | tee $out/log.txt
        ${postScript}
      '';
      dontInstall = true;
    };
in
{
  lint = run {
    name = "fe-lint";
    script = "make lint";
  };
  test = run {
    name = "fe-test";
    script = "make test";
  };
}
