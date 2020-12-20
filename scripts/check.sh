#!/bin/bash

# All default features + FFI
cargo check --features ffi

# All combos that include at least `cli` and one pf `http`/`electrum`
feature_combos="CH CE CEH CET CHT CHW CEW CEHT CEHW CETW CHTW CEHTW"

# Some simple combos with `ffi` and no `cli`
feature_combos="$feature_combos FE FHT"
# TODO test more `ffi` and `extra` combos

for features in $feature_combos; do
  features=`echo $features | sed 's/H/http /; s/E/electrum /; s/W/webhooks /; s/T/track-spends /; s/C/cli /; s/F/ffi /;'`
  echo "Checking $features"
  cargo check --no-default-features --features "$features"
done
