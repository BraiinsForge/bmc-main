#!/bin/sh
exec /run/current-profile/bin/bmc-wasm-thin-v@protocolMajor@ \
    --wasm "${0%/*}/../lib/wasm/@name@.wasm" \
    --asset-root "${0%/*}/../lib/assets" \
    "$@"
