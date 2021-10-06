#!/bin/bash

cargo check
cargo check --all-features

# All combos include at least `cli` and one of `http`/`electrum`
feature_combos="CH CE CEH CET CHT CHW CEW CEHT CEHW CETW CHTW CEHTW CEHTWP"

for features in $feature_combos; do
  features=`echo $features | sed 's/H/http /; s/E/electrum /; s/W/webhooks /; s/T/track-spends /; s/C/cli /; s/P/proxy /;'`
  echo "Checking $features"
  cargo check --no-default-features --features "$features"
done
