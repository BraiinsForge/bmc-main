## bmc mock

A mock that is runnable on regular personal computers, simulating the backend of the application running on the
hardware. Currently not implementing the compositor.

Start mock by running:

```shell
cargo run
```

On a first run, it creates a directory in `~/.local/share/bmc-mockup` with the following structure:

```
bmc-mockup
├─── mockfs
└─── www
        ├─── bmc
        └─── var
```

### Web assets

Web assets are generated independently using the Nix build system. It is required to perform this build prior to
executing the BMC service. To generate the assets, run the following command:

```shell
nix build .#web-assets --out-link ~/.local/share/bmc-mockup/www
```

### Command-Line Options

- `--address` — Set the server address. By default, it runs on 127.0.0.1:6060
- `--www-path` — Set the path to the web content directory. Default value: `~/.local/share/bmc-mockup/www`
- `--www-var-path` — Override the path to the web variable content directory. Default value:
  `~/.local/share/bmc-mockup/www/var`
- `--mockfs-path` — Set the path to a writable directory for mockup config files. Default value:
  `~/.local/share/bmc-mockup/mockfs`
- `--mockfs-template` — Path to a directory where the mock files should be copied from. Default value:
  `./bmc-mock/mockfs-template/bmc100`

Example of running a mockup with a custom path to the web content directory:

```shell
cargo run -- --www-path /home/user/www
```
