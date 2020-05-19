#!/bin/bash

# All combos that include at least HTTP or Electrum
feature_combos='H E EH ET HT HW EW EHT EHW ETW HTW EHTW'

for features in $feature_combos; do
  features=`echo $features | sed 's/H/http /; s/E/electrum /; s/W/webhooks /; s/T/track-spends /;'`
  echo "Checking $features"
  cargo check --no-default-features --features "$features"
done
