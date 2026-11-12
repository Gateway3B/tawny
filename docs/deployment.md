# Tawny deployment

Tawny is a Rust full-stack server plus three supporting containers. GitHub
Actions publishes two image tags each for the two images Tawny itself owns,
after a successful build on `main`:

- `ghcr.io/gateway3b/tawny:latest` and `:sha-<commit>`
- `ghcr.io/gateway3b/tawny-extractor:latest` and `:sha-<commit>`

The sidecar is published too, not just the app: `compose.yml` at the repository
root builds it from `docker/extractor`, so a pull-only deployment with only the
app image would have nothing to extract streams with.

It is named for the job rather than for yt-dlp, which is one of several things
inside it: yt-dlp itself, a Node runtime, the bgutil PO-token plugin, and a
small HTTP adapter exposing the two operations Tawny needs.

The other two services come from upstream images and need nothing published:
SurrealDB, and the bgutil PO-token provider.

A server always runs as this Compose stack. Someone trying Tawny without one
can keep their library in the desktop or Android app instead and move it to a
server later; see [device-backend.md](device-backend.md).

## One-time GitHub setup

1. Merge the deployment workflow and let its first `main` run finish.
2. On the gateway3B GitHub profile, open **Packages**, then each of **tawny**
   and **tawny-extractor**.
3. Open **Package settings** and change the package visibility to **Public**.

Public GHCR packages can be pulled without credentials. Package visibility is
separate from repository visibility, so verify this once after the first images
are published. If you keep them private, the Portainer host needs
`docker login ghcr.io` with a token carrying `read:packages`.

## Native mobile artifacts

Versioned releases that installed apps update themselves from are published
from a workstation: see [releases.md](releases.md). What follows is for
one-off builds.

Native compilation is intentionally separate from image publication and pull
request CI. Run **Build Mobile Artifacts** manually, choose Android, iOS, or
both, and enter the public HTTPS origin of the Tawny server the app should use.
Android runs on Ubuntu and uploads an APK and AAB; iOS runs on macOS and uploads
an IPA. Artifacts expire after 14 days. Both jobs install `dx` from the
Gateway3B Dioxus fork, which must match the libraries the app builds against.

Every run numbers its build with the workflow run number, so each store upload
is higher than the last. Signing is optional and turns on when its repository
secrets exist; without them the artifacts are unsigned:

| Platform | Secrets | Effect |
| --- | --- | --- |
| Android | `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | Signs with the Play upload key (use Play App Signing). |
| iOS | `IOS_DISTRIBUTION_CERTIFICATE_BASE64`, `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD`, `IOS_APPSTORE_PROFILE_BASE64` | Builds with `--appstore`: an Apple Distribution certificate (`.p12`) and an App Store provisioning profile. |

The Android keystore and passwords are written to the runner only: the job
appends `[android.signing]` to its copy of `Dioxus.toml` and decodes the
keystore outside the checkout. The generated Gradle project holds the
passwords, so only `artifacts/` is uploaded; never upload the build directory.
The iOS certificate goes into a temporary keychain that is deleted when the job
ends. See [mobile.md](mobile.md#builds) for what a first store submission still
needs.

## Portainer stack

1. In Portainer, open the target environment and select **Stacks** → **Add stack**.
2. Give the stack a name such as `tawny`.
3. Choose **Web editor** and paste the contents of
   [`compose.prod.yml`](../compose.prod.yml), or choose **Git repository** and
   point Portainer at this repository with `compose.prod.yml` as the Compose
   path.
4. Set the stack environment variables below.
   [`.env.prod.template`](../.env.prod.template) lists every one the stack
   reads, with the same comments; copy it to `.env.prod` on a plain Docker host
   instead of pasting into Portainer.
5. Deploy the stack.

### Environment

Two variables have no default, and the stack refuses to start without them
rather than doing something worse quietly:

| Variable | Why it has no default |
| --- | --- |
| `SURREALDB_PASSWORD` | A deployed database with the password `root` is worse than a stack that will not start. |
| `TAWNY_PUBLIC_URL` | The origin browsers reach this instance on. Playback proxy URLs and WebSub lease callbacks are built from it, so a wrong value surfaces as broken playback rather than as a configuration error. |

Optional:

| Variable | Default | Notes |
| --- | --- | --- |
| `TAWNY_IMAGE_TAG` | `latest` | Pin to `sha-<commit>` to hold a known-good build. |
| `TAWNY_PORT` | `8080` | Host port. Only Tawny publishes one. |
| `TAWNY_WEBSUB_CALLBACK_URL` | derived | Set when the public URL is not where YouTube should call back. |
| `TAWNY_WEBSUB_SECRET` | generated | Generated and kept on the app volume if unset. Fine only while that volume survives: replace it and every existing subscription fails its signature check until it renews. |
| `YTDLP_CONCURRENCY` | `2` | Simultaneous extractions. |
| `TAWNY_SECURE_COOKIES` | `true` | Set `false` only for a LAN server reached over plain `http://`: browsers drop a `Secure` session cookie sent over http, so every visit would become a new guest. The cookie then crosses the network unencrypted. |
| `TAWNY_SURREALDB_VOLUME` | `surrealdb-data` | Where the database volume lives. A bare name is a Docker named volume; an absolute path is a bind mount. |
| `TAWNY_APP_VOLUME` | `tawny-data` | The same, for the app's own data. |
| `TAWNY_PROXY_NETWORK` | `tawny-proxy` | An existing Docker network to put Tawny on, so a reverse proxy in it reaches Tawny as `tawny:8080`. |
| `TAWNY_PROXY_NETWORK_EXTERNAL` | `false` | Set `true` alongside the above. Naming an existing network without this makes Compose try to create it and fail. |

Unlike the development `compose.yml`, only Tawny publishes a port. SurrealDB,
the PO-token provider and the extractor reach each other over the stack's network;
development binds them to `127.0.0.1` only so a host `dx serve` can reach them,
which is not a deployment need.

## Updating

`pull_policy: always` checks for a newer image when the stack is redeployed or a
container is recreated. A restart policy does not, by itself, replace a running
container when a new image is pushed - use Portainer's stack webhook or another
updater to trigger a redeploy after GitHub publishes.

From a shell instead:

```sh
docker compose -f compose.prod.yml pull
docker compose -f compose.prod.yml up -d
```

## State

Two named volumes hold everything worth keeping:

- `surrealdb-data` - the catalog, and every account's library
- `tawny-data` - extractor caches, the generated WebSub secret, and the
  group suggestion model (90 MB, fetched from Hugging Face the first time
  someone asks for suggestions)

Back up `surrealdb-data`. Losing `tawny-data` costs a re-warmed cache, a new
WebSub secret, which resubscribes on the next renewal, and a fresh model
download.

`TAWNY_SURREALDB_VOLUME` and `TAWNY_APP_VOLUME` place each one. A bare name is
a Docker named volume, which Docker keeps under `/var/lib/docker/volumes`; an
absolute path makes it a bind mount instead, which is what you want when the
stack directory is managed for you, as it is under Portainer, or when the data
belongs on a specific disk. Switching an existing stack between the two does
not move anything: copy the contents across first, or the containers come up
empty.

Stop the app before restoring a database backup and restart it afterward.
Backups and storage retention are managed by the individual running the server.

## Reverse proxy

Put TLS in front of Tawny. `TAWNY_PUBLIC_URL` must be the public HTTPS origin:
WebSub stays disabled on a loopback URL, and playback proxy URLs are built from
it. Tawny speaks plain HTTP and needs no WebSocket upgrade; the only `ws://` in
the stack is SurrealDB, which never leaves it.

A server only ever reached on the local network can skip TLS. Set
`TAWNY_SECURE_COOKIES=false` then, or browsers drop the `Secure` session
cookie and every visit starts a new guest. The iOS app reaches local
addresses over plain http (`NSAllowsLocalNetworking`); anything over the
internet needs HTTPS.

There are two ways to connect a proxy, and the stack does not care which:

**Over the published port.** Leave the networking alone and point the proxy at
the Docker host on `TAWNY_PORT`. Nothing to configure here, and it works with a
proxy that is not in Docker at all.

**Over a shared network.** Put Tawny on the proxy's own network and it becomes
reachable as `tawny:8080`, with no host port to expose:

```dotenv
TAWNY_PROXY_NETWORK=npm_default
TAWNY_PROXY_NETWORK_EXTERNAL=true
```

Both variables move together, and the network must already exist. Left unset,
the stack owns an ordinary network of its own and needs no knowledge of anyone's
proxy, so this costs a consumer nothing.

Attaching the container by hand after the stack is up works for one container
lifetime only. A recreated container gets the networks in its Compose
configuration and nothing else, and `pull_policy: always` recreates on every
redeploy, so it would come back off the proxy network each time.

If the WebSub callback is on a second hostname, it must reach
`/api/v1/websub/youtube`, which answers **GET** for the hub's subscription
verification and **POST** for notifications. Behind a CDN that filters
non-browser traffic, exempt that path or the subscription silently never
verifies.
