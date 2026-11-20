# Build and test

The build uses the pinned Rust toolchain, Node.js 22.12 or later, npm, and the Xcode command line tools. The runtime setup verifies the pinned upstream llama.cpp archive. You can pass a previously downloaded archive as its first argument for offline setup.

```sh
scripts/prepare-runtime.sh
npm ci
npm run desktop
```

To make and verify the distributable bundle and the website download, run `scripts/release.sh`. The standalone landing page is `website/index.html`. It loads its fonts and paper locally. On disk and on localhost the download uses the adjacent Mac archive. The public page links to the tagged release. To publish it, copy the page, both fonts and the resolved paper image into the root of the `gh-pages` branch. Include the asset notices from `licenses/` and preserve `.nojekyll`. Publish the matching Mac release before updating the download link.

```sh
cargo test --manifest-path src-tauri/Cargo.toml
cargo build --manifest-path src-tauri/Cargo.toml
npm run test:ui
```

The UI test uses Google Chrome, an installed Ollama llama3.2:3b model, a temporary notebook, and the debug build of the Rust core. Additional integration checks exercise local inference, the packaged command-line agent server and loopback HTTP. Build the release app before running them.

```sh
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
```

To exercise the publisher download in the browser suite, set `SUMI_TEST_MODEL_DOWNLOAD=1`. This optional test downloads the strongest supported tier that fits the available RAM and storage from Hugging Face and removes its temporary notebook and weights afterward. On repeat runs, set `SUMI_TEST_GGUF_PREFIX` to an existing GGUF with the same filename to reuse its leading bytes. The test still interrupts a real transfer, resumes it from the publisher, verifies the full digest and runs an inference.

The source is MIT licensed. Bundled fonts and inference runtime notices are under `licenses/`. See the [verification record](requirements-review.md) for the evidence and outstanding criteria, and [Contributing](../CONTRIBUTING.md) for feedback and code changes.


## Publishing a signed model list

Official builds include the model list and its verification key under `catalog/`. They check the signed manifest on the `model-catalog` branch weekly while idle. To update it, increment the revision in `catalog/payload.json`, run `swift scripts/sign-catalog.swift catalog/payload.json catalog/models.json` on the release Mac, review the changes and publish the signed manifest to that branch. The signing key lives in macOS Keychain. A publisher may set `SUMI_CATALOG_URL` to its HTTPS manifest URL and `SUMI_CATALOG_PUBLIC_KEY` to its base64-encoded 32-byte Ed25519 public key at compile time. The envelope contains `payload` and `signature`, both standard base64. Sign the exact UTF-8 JSON payload bytes. The payload contains an integer `revision` above 1 and an ordered `models` array of Ollama `name` and byte-size `bytes` records. The app rejects invalid signatures and older revisions. Users can disable weekly checks. Models download only on request.

The redistributable source archive is built by `python3 scripts/source-archive.py` and written to `release/Sumi-source.zip`. The release script runs this step automatically.

The weekend walkthrough is opt-in. With the local model and browser prerequisites above, run `SUMI_LIVE_SEARCH=1 npm run test:ui -- -g 'a weekend plan'`. It creates a synthetic notebook, searches public listings through the keyless Exa endpoint, checks the source and memory mark, retires the preference, and reopens the notebook. Results vary with the date and available listings.

To distribute with Apple notarization, set `SUMI_SIGN_IDENTITY` to the Developer ID Application identity and `SUMI_NOTARY_PROFILE` to its Keychain notarytool profile before running `scripts/release.sh`. The script signs the embedded runtime, submits the app, staples the ticket, and verifies the extracted archive. With neither variable set, it builds an ad-hoc signed copy.
