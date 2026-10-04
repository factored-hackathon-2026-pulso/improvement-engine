# contracts/product (FX22p)

Goldens of the Product consumer contract (alias resolution + `release.published` / `release.rolled_back` rows),
copied from `platform-contract/release-contract/examples/valid` and kept identical by `test_product_contract.py`,
which also validates them against the release-contract schemas and drives `platform-sim` with them.
Platform side is simulated (product-consumer) until EXT-2; catalog 1.1.0 is unchanged (release.* are quarantined).

    python -m pytest contracts/product -p no:cacheprovider
