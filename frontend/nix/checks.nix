{ pkgs, src, yarnFiles }:

let
  run =
    { name
    , script
    , postScript ? ""
    ,
    }: pkgs.stdenv.mkDerivation {
      name = "ii-fe-check-${name}";
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
  # Linters
  js = run {
    name = "js";
    script = "make lint-js";
  };
  yarn = run {
    name = "yarn";
    script = "make lint-yarn";
  };
  styles = run {
    name = "styles";
    script = "make lint-styles";
  };

  # Tests
  test-static = run {
    name = "test-static";
    script = "make ci-test-static";
  };
  test-unit = run {
    name = "test-unit";
    script = "make ci-test-unit";
    postScript = "cp -r coverage $out/";
  };
}
