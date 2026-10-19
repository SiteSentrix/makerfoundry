# MakerFoundry's API for the apps (MF-7)

**Status:** 2026-10-19. Built and tested on MakerFoundry's side (`apps/makerfoundry/src/api.rs`).
The apps' side (a **Publish to MakerFoundry** command) is StitchFoundry's and LightFoundry's work,
in their repositories, under their rules.

## How publishing from an app works
1. The person is signed in to their **Foundry Account** in the app (the app holds the account
   service's session token, `sfs_…`, as it does for AI credits).
2. The app exports the design (StitchFoundry: the embroidery file in the format the person picks;
   LightFoundry: SVG or DXF) and sends it, with any photos, to MakerFoundry with that token.
3. MakerFoundry checks the token with the account service (`POST /v1/account`), reads and checks
   the file exactly as an upload on the site, keeps it as a **draft**, and answers with its page.
4. The app opens that page in the browser. There the maker adds the title, description, tags,
   licence and price, and publishes: publishing (and agreeing to the Seller Terms) always happens
   on the site. If the browser isn't signed in to MakerFoundry, the page asks for the Foundry
   Account sign-in and comes back to the draft.

MakerFoundry uses the token for that one request and never keeps or logs it.

## `POST https://makerfoundry.app/api/v1/designs`
Headers: `Authorization: Bearer <Foundry Account session token>`,
`Content-Type: application/json`. Body:

```json
{
  "file_name": "rose.pes",
  "file": "<the design file, base64 (standard alphabet, with padding)>",
  "photos": ["<JPEG, PNG or WebP of the stitched or cut result, base64>"],
  "app": "StitchFoundry 0.9.0"
}
```

- `file_name` decides how an embroidery file is read (its extension); only its last part is kept.
- `photos` (optional): up to 6. They're saved again without their metadata (location, camera).
- `app` (optional): the app and its version, for MakerFoundry's log.
- Limits are the site's: embroidery files up to 4 MB, laser files up to 8 MB, 64 MB in all
  (before base64), 50 designs a day and 500 kept per account.

Answers (JSON):

| Status | Body | Means |
|---|---|---|
| 201 | `{"id": 42, "url": "https://makerfoundry.app/designs/42", "kind": "embroidery", "format": "PES"}` | Kept as a draft: open `url` in the browser |
| 400 | `{"error": {"message": "…"}}` | Not JSON, no file name, not base64, too many photos |
| 401 | same | No token, or the sign-in has ended: sign in again |
| 413 | (no body) | Bigger than the limit |
| 422 | same | The file or a photo couldn't be read (the message says why, for the person) |
| 429 | same | Too many designs today, too many kept, or too many failed sign-ins |
| 503 | same | MakerFoundry or Foundry Accounts can't be reached, or keeping designs is off |

Every `error.message` is a sentence to show the person as it is.

## `GET https://makerfoundry.app/api/v1`
`{"name": "MakerFoundry", "version": "0.1.0", "designs": "/api/v1/designs", "max_upload_bytes":
67108864, "max_photos": 6}`: for an app to check it's talking to MakerFoundry and read the limits.

## From a web app (browsers)
The web apps' sites may call the API from the browser: `MF_API_ORIGINS` (default
`https://stitchfoundry.app`; add LightFoundry's when it has one) lists them. A browser's
preflight (`OPTIONS`) is answered for those sites only, allowing `POST` with the `authorization`
and `content-type` headers. No cookies are used or allowed. Desktop apps don't need any of this.

## For StitchFoundry (a note for its chat)
StitchFoundry's `sf_core::ai::cloud::request(server, path, token, body)` already builds the
request: `request("https://makerfoundry.app", "/api/v1/designs", Some(&token), &body)` with the
body above, sent like `cloud::report` (`prompt::post_plain`), then `app.open_url(&answer["url"])`.
Suggested shape, by StitchFoundry's rules (everything is a command, wasm must build, `cargo xtask
ci` green, its own task id):

- A command `embroidery.publishToMakerFoundry` (menu: File › Publish to MakerFoundry…), enabled
  when there's an embroidery design and the person is signed in to Foundry Account; params
  `{format}` (default PES) and optionally `{photos}` from the sew-out check's photos.
- It exports the active design with the chosen writer, base64-encodes it, sends it, and on `201`
  opens `url`; on an error it shows `error.message`.
- A setting for MakerFoundry's address (default `https://makerfoundry.app`, a local address for
  testing, checked with `cloud::clean_server`).
