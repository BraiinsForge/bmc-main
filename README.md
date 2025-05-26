Braiins clock

## Build frontend

```
cd ./frontend
nix-shell
yarn install
make build
```

## Run mock with built frontend assets

```
cargo run --bin bmc-mock -- --address=0.0.0.0:6070 --www-path=./frontend/dist
```
