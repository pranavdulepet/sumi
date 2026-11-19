# Sumi 墨

Sumi is a private notebook for Apple Silicon Macs running macOS 14 or later. It records what it guessed before you tell it what you did. Over time, those comparisons give you evidence for deciding which beliefs to keep.

[Watch the ten walkthroughs](docs/walkthroughs.md) · [Download for Mac](https://github.com/pranavdulepet/sumi/releases/latest/download/Sumi-macOS.zip)

## Open your notebook

Download **Sumi-macOS.zip** from the [latest release](https://github.com/pranavdulepet/sumi/releases/latest), unzip it, and move Sumi to Applications. Move the app to its lasting location before connecting agents. This build uses an ad-hoc signature. A rebuilt copy may trigger macOS Keychain authorization for the existing notebook. The release has no Apple notarization ticket.

Draw a mark or choose the keyboard-accessible stroke. Pick an installed local model, supply a GGUF file, or allow the bundled runtime to download its listed model. A clean notebook works without model weights. Each conversation source asks for your permission. You can skip imports entirely.

The first reading prioritizes replies from the last 90 days. For a long coding session, it starts near the end so your latest decisions can reach the notebook first. Sumi picks up the rest while your Mac is idle and plugged in.

When you import a ChatGPT archive, Sumi reads the conversation branch selected at export time. If the export omits the selection, Sumi reads the shared history up to the first fork. To replace a previously imported archive and its learned history, open **Seal**, remove that source, confirm **Forget source**, then import the archive again.

Write a real decision on the Desk and choose **Seal a guess**. Later, enter what you decided, open the seal, and judge the guess. The Mirror records your verdict. Open **Places** or press ⌘1–4 to move among the Desk, Scroll, Mirror and Seal. Option-Space opens a fresh sheet inside Sumi and works globally when macOS grants the shortcut. You can also choose **New Desk Sheet** from Sumi’s menu bar dot.

When you return the next morning, the Desk opens a blank sheet. Earlier writing stays under **Sheets**. A waiting question names the agent that asked it. Open that line to answer in the Seal, where the agent can collect your reply.

## Files you own

The default notebook lives in `~/Sumi`. Sheets and Scroll entries are readable Markdown. Scroll changes have private local Git history. Machine observations, settings, scores and vectors live in an encrypted SQLCipher database under `.sumi`. The decryption key and optional provider credentials live in macOS Keychain. Reading the Markdown requires access to the user's files. FileVault protects the volume while locked. Keep your login Keychain with your notebook backup. The notebook path identifies its key.

The Seal connects an agent after confirmation and writes one named MCP entry to its configuration. It starts with scoped read access. Some questions can be delegated after enough reviewed answers. Open the agent in the Seal for a line you can add to its instructions. The Seal counts how many automatic answers you reviewed and corrected. Its Repair connection control restores the launcher after an app move and preserves the agent’s permissions. Revoke access disables the credential and removes the launcher Sumi wrote. If another program already uses the name `sumi`, the Seal shows the separate name it registered. The HTTP transport listens on loopback. Installed agents use stdio.

On a Mac without an installed model, the Qwen download shows its progress and resumes interrupted transfers on retry. Sumi checks the complete file against the publisher’s pinned SHA-256 digest before activating it.

The app includes a local llama.cpp server. Model weights are separate. Selecting Ollama or LM Studio uses its local service. Hosted overnight reflection, optional blind prediction and Jev judgments require explicit configuration and a nonzero monthly request allowance. A request limit counts calls. Check the provider for currency limits.

## Build and test

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

The source is MIT licensed. Bundled fonts and inference runtime notices are under `licenses/`. See the [verification record](docs/requirements-review.md) for the evidence and outstanding criteria, and [Contributing](CONTRIBUTING.md) for feedback and code changes.


## Publishing a signed model list

Local builds use the included list of verified local model identifiers. A publisher may set `SUMI_CATALOG_URL` to its HTTPS manifest URL and `SUMI_CATALOG_PUBLIC_KEY` to its base64-encoded 32-byte Ed25519 public key at compile time. The envelope contains `payload` and `signature`, both standard base64. Sign the exact UTF-8 JSON payload bytes. The payload contains an integer `revision` above 1 and an ordered `models` array of Ollama `name` and byte-size `bytes` records. The app rejects invalid signatures and older revisions. Users can disable weekly checks. Models download only on request.

The redistributable source archive is built by `python3 scripts/source-archive.py` and written to `release/Sumi-source.zip`. The release script runs this step automatically.
