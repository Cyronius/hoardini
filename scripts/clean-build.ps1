# Drops build output not touched in $Days days; run after merging a branch.
param([int]$Days = 3)
cargo sweep --time $Days
