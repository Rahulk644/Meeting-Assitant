# Self-hosted CPU transcription worker

This stack runs the pinned `transcribe.cpp` server with one loaded Nemotron 3.5 model and two named
streaming sessions. It can run on a private Linux machine, a VM/VPS from any provider, or a local Docker
host with enough memory. Hostinger KVM 1 is one measured reference profile, not a product dependency.

My Assistant already has an in-process on-device provider on macOS. Running this container on the same Mac
is useful only for protocol/deployment testing; it is not expected to be lighter than the native path.

## Trust boundary

- The container port is published only on host loopback (`127.0.0.1:18091`) by default.
- A bearer token is read from a read-only file and never belongs in Compose, git, process arguments, or logs.
- Remote access must use the restricted persistent SSH tunnel below or a separately qualified private VPN.
- The service does not persist audio or transcript text. Docker logs contain lifecycle/errors only.
- The model file is pinned and must match the SHA-256 already recorded in `speech_models.rs`.

## Requirements

- A Linux Docker Engine with Compose v2, or Docker Desktop for local protocol testing.
- Enough RAM for the selected model plus runtime headroom; the measured Q8 reference used about 1.27 GB.
- A model artifact accepted by the pinned `transcribe.cpp` revision.
- A random bearer token of at least 32 characters.

The image enables `GGML_NATIVE`, so build it on the machine class where it will run. Rebuild after moving to
a materially different CPU. GPU runtimes are outside this reference deployment.

## Host preparation

Create `models/` and `secrets/` beside `compose.yaml`. Copy `.env.example` to `.env` only if you need to
change the model filename, bind port, session count, or resource envelope. Never store the bearer token in
`.env` or Compose. Place:

- `models/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf`
- `secrets/stt-token` containing one random token of at least 32 characters, owned by numeric UID/GID
  `10001:10001` with mode `0400`

Verify the model:

```sh
sha256sum models/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf
```

Expected SHA-256:

```text
b94545b313b3223fda7b2857a52681da813935c2127643d1e9ff0c23d988089c
```

Build and start from the repository root so Docker receives only the allowlisted build inputs:

```sh
docker compose -f deploy/self-hosted-stt/compose.yaml config --quiet
docker compose -f deploy/self-hosted-stt/compose.yaml build
docker compose -f deploy/self-hosted-stt/compose.yaml up -d
docker compose -f deploy/self-hosted-stt/compose.yaml ps
```

The CPU-native image must be rebuilt if it moves to a VPS exposing a different CPU instruction set.

## Benchmark

Use a 16 kHz mono signed-16 WAV. First test maximum throughput, then real-time scheduling:

```sh
python3 benchmark_streams.py --token-file secrets/stt-token --wav sample.wav --streams 1 --mode max
python3 benchmark_streams.py --token-file secrets/stt-token --wav sample.wav --streams 2 --mode max
python3 benchmark_streams.py --token-file secrets/stt-token --wav sample.wav --streams 2 --mode realtime
```

The script reports timing only and deliberately does not print or store transcript text.

## Provider-neutral configuration

The checked-in Compose file does not contain a Hostinger address, SSH account, DNS name, or credential.
Configuration in `.env.example` supports any qualified host and remains loopback-bound by default. For a
different model, change `STT_MODEL_FILE` only after verifying that the pinned runtime supports it and record
its source, license, size, checksum, memory, latency, and language behavior.

The HTTP contract is intentionally small:

- unauthenticated `GET /livez` for container health on the same host;
- bearer-authenticated `GET /health` for readiness/capabilities;
- bearer-authenticated `POST /inference` for bounded multipart WAV inference; and
- bearer-authenticated named streaming-session routes for compatible incremental models.

This is a My Assistant/transcribe.cpp contract, not a universal OpenAI, Deepgram, or WhisperLive endpoint.
Other engines need a small explicit adapter and capability tests rather than pretending their protocols are
interchangeable.

## Qualified Hostinger KVM 1 reference

The current Hostinger KVM 1 qualification used one vCPU, 4 GB RAM, 2 GB swap, the pinned Q8 model, and
two 560 ms sessions. The worker held one model copy and serialized compute as required by transcribe.cpp:

| Workload | Wall time | Aggregate audio | Throughput | Maximum live scheduling lag |
|---|---:|---:|---:|---:|
| One stream, maximum rate | 4.649 s | 11.0 s | 2.366× real-time | n/a |
| Two streams, maximum rate | 7.882 s | 22.0 s | 2.791× real-time | n/a |
| Two streams, real-time cadence | 11.560 s | 22.0 s | 1.903× real-time | 288.6 ms |

The container used about 1.27 GB after the dual-stream run. This qualifies simultaneous microphone and
meeting-audio ingestion on this host class, not arbitrary extra workers or two remote speakers inside one
mixed playback track.

## Restricted SSH tunnel from macOS

For a remote host, create a dedicated SSH key on the Mac and authorize its public half for an account that
has no general shell role. Restrict that `authorized_keys` entry to this destination:

```text
restrict,port-forwarding,permitopen="127.0.0.1:18091" ssh-ed25519 … my-assistant-stt-tunnel
```

Then install the checked-in LaunchAgent (replace the example hostname and user with your provider):

```sh
./install_macos_tunnel.sh \
  --host speech.example.com \
  --identity /absolute/path/to/my_assistant_stt_worker \
  --user myassistant-stt
```

The script binds `127.0.0.1:18091` on the Mac, uses strict host-key checking, keeps the connection alive,
and verifies `/livez`. In My Assistant, open **Settings → Meetings → Meeting transcription compute**, choose
**Private server**, enter `http://127.0.0.1:18091`, paste the bearer token, and select **Test & use private
server**. The app verifies authenticated `/health` before saving the token to macOS Keychain.

For a same-machine Linux/Docker deployment, skip SSH and keep the loopback URL. For a remote service without
SSH or a qualified private VPN, terminate trusted HTTPS at the host and configure that `https://` URL; do not
publish the worker's plain HTTP port to the internet.
