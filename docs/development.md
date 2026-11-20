# Build and test

The build uses the pinned Rust toolchain, Node.js 22.12 or later, npm, and the Xcode command line tools. The runtime setup verifies the pinned upstream llama.cpp archive. You can pass a previously downloaded archive as its first argument for offline setup.

```sh
scripts/prepare-runtime.sh
npm ci
npm run desktop
```

To make and verify the distributable bundle and the website download, run `scripts/release.sh`. The standalone landing page is `website/index.html`. It loads its fonts locally and works when opened on disk.

```sh
cargo test --manifest-path src-tauri/Cargo.toml
cargo build --manifest-path src-tauri/Cargo.toml
npm run test:ui
```

The UI test uses Google Chrome, an installed Ollama llama3.2:3b model, a temporary notebook, and the debug build of the Rust core. Additional integration checks exercise local inference, the packaged command-line agent server and loopback HTTP. Build the release app before running them.

```sh
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
```

To exercise the publisher download in the browser suite, set `SUMI_TEST_MODEL_DOWNLOAD=1`. This optional test downloads 1.12 GB from Hugging Face and removes its temporary notebook and weights afterward.

The source is MIT licensed. Bundled fonts and inference runtime notices are under `licenses/`. See the [verification record](requirements-review.md) for the evidence and outstanding criteria, and [Contributing](../CONTRIBUTING.md) for feedback and code changes.


## Publishing a signed model list

Local builds use the included list of verified local model identifiers. A publisher may set `SUMI_CATALOG_URL` to its HTTPS manifest URL and `SUMI_CATALOG_PUBLIC_KEY` to its base64-encoded 32-byte Ed25519 public key at compile time. The envelope contains `payload` and `signature`, both standard base64. Sign the exact UTF-8 JSON payload bytes. The payload contains an integer `revision` above 1 and an ordered `models` array of Ollama `name` and byte-size `bytes` records. The app rejects invalid signatures and older revisions. Users can disable weekly checks. Models download only on request.

The redistributable source archive is built by `python3 scripts/source-archive.py` and written to `release/Sumi-source.zip`. The release script runs this step automatically.
