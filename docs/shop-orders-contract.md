# Ordering from shops through StitchHub: the message contract (issues #2 and #1)

**Status:** 2026-11-01. **StitchHub's form** (issue #53, from StitchHub's message of 2026-11-01:
stitchhub#19 and #34). StitchHub owns this contract and implements it; this page is MakerFoundry's
summary of it, and MakerFoundry's side (MF-80 to MF-86) is built to it and switched off. The owner
decided (#2, `docs/stitchshop-ordering-audit.md`): customers order physical goods from shops
through MakerFoundry, live; MakerFoundry takes the payment and keeps 10%, and the shop pays the
card fee; orders travel through **StitchHub**. StitchHub's own fixtures
(`test/fixtures/sync-orders/`) and StitchShop's check against a real hub
(`online-realhub.check.ts`) are the reference; ours, in `docs/contract/shop-orders/`, are copies in
the same form.

## Who's who
| Party | Role | Proves itself with |
|---|---|---|
| **MakerFoundry** (`service:makerfoundry`) | Takes the customer's order and payment; sends orders and the customer's answers; shows the customer the shop's status and proofs | A service key the hub admin issues (`svk_…`, `MF_STITCHHUB_KEY`, a Railway secret), sent as `Authorization: Bearer svk_…`; and `order.placed` signed with its Ed25519 key (`MF_STITCHHUB_SIGNING_KEY`, a Railway secret; the hub admin registers the public half) |
| **StitchHub** (`hub`) | Lists shops that take orders; queues and delivers messages both ways; holds art files briefly; sends `message.expired` | n/a |
| **StitchShop** (`shop:<hub id>`) | Opts in; publishes its catalogue; takes orders into a decorating order; sends status and proofs | Its install key, as for backups and the inbox |

A shop opts in on StitchShop (Settings › Online orders › "Take orders from MakerFoundry"). Until it
does, the hub never lists it and refuses orders for it.

## The handshake (versions)
Every request carries `Authorization: Bearer svk_…` and `X-Client: makerfoundry/<version>`.

- `GET /v2/hello` is public: `{ "min_client": { "makerfoundry": "…" }, "limits": {…},
  "deprecated": [{ "type": "…", "version": 1, "until": "…" }], "keys": […] }`. A MakerFoundry older
  than `min_client` stops sending and says so (`mf_hub_state.too_old`); any call answered `426`
  means the same.
- `POST /v2/hello` with `{ "app": "makerfoundry", "version": "<version>", "types": {
  "catalog": [1], "order.status": [1], "proof.sent": [1], "order.cancel": [1],
  "message.expired": [1] } }`: the types MakerFoundry reads. Until it's said, `GET /v2/messages`
  answers `409 hello_first`; MakerFoundry says hello every round, and again on a `hello_first`.
- `deprecated` entries are logged once a day.
- Each message type carries its own `version`. Within a version, fields are only ever added, and
  readers ignore fields they don't know. A breaking change is a new version, offered alongside the
  old until its `until` date.

## Shops
`GET /v2/shops` → `{ "shops": [{ "id": "shop:<hub id>", "name": "…", "ready": true }] }`: the shops
that opted in. MakerFoundry offers a shop only while the hub lists it as `ready` (and its
catalogue is kept, and its Foundry Account can be paid); the part after `shop:` is its page's
address (`/shops/<id>`).

## The envelope
Every message, both ways:

```json
{ "id": "0192f0c4-…", "type": "order.status", "version": 1,
  "conversation": "mf-8f3a1", "seq": 4,
  "from": "shop:mainst", "to": "service:makerfoundry",
  "created_at": "2026-11-02T15:04:05.123Z", "expires_at": "2026-11-09T15:04:05Z",
  "body": { … }, "sig": { "v": 1, "sig": "…" } }
```

- `id`: a UUID made by the sender. **The receiver de-duplicates by `id`**: the same message twice
  is applied once. Senders retry with the same `id`.
- `conversation`: 1 to 120 of `A-Z a-z 0-9 . _ : -`. For an order it's MakerFoundry's reference
  (`mf-…`); a shop's catalogue is in the conversation `catalog`.
- `seq`: per sender per conversation, from 1. The receiver applies a conversation's messages in
  `seq` order, holding a later one until the earlier arrives; a gap older than 24 hours is logged.
- `created_at` must be within 10 minutes of the hub's clock: MakerFoundry stamps a message again
  when it sends it after a wait (8 minutes or more since it was made or last tried).
- `from`: on MakerFoundry's messages absent (or exactly `service:makerfoundry`); the hub sets it
  on delivery. A test deploy has its own key and name (`service:makerfoundry-test`, set with
  `MF_STITCHHUB_SERVICE`), and then that name stands wherever this page says
  `service:makerfoundry`, the signed text's sender line included. The hub's own messages are `from: "hub"`; shops' are `shop:<hub id>`.
- `expires_at`: when the hub gives up delivering (MakerFoundry sets 7 days for orders, 14 for
  proofs).
- `sig`: on `order.placed` only (below).
- Delivered messages also carry the hub's `delivery_seq`, which readers ignore.

## Signing `order.placed`
MakerFoundry signs every `order.placed` with Ed25519 (RFC 8032) and sends `sig: { "v": 1, "sig":
"<base64 signature>" }`; unsigned, or signed by a key the hub doesn't know, it's refused (`403`).
The signed text is these lines joined by a newline: `SBHUB1`, `service:makerfoundry`, `to`,
`conversation`, `seq`, `id`, `type`, `version`, `created_at`, `expires_at` (empty when absent),
then the SHA-256 (hex) of the body's canonical JSON (keys sorted at every level by UTF-16 code
units, no whitespace). `src/shop_orders/sign.rs` builds it, with RFC 8032's test vector in its
tests.

## Delivery: the queue
- **Sending:** `POST /v2/messages` with the envelope → `202 { "queued": true, "state": "…" }`.
  The same `id` with the same content again → `202 { "duplicate": true }`. `409 id_reused` (the
  hub has that id already) MakerFoundry counts as delivered: an earlier try got through and its
  answer was lost. `409 seq_taken` and other `4xx` are final (kept with why; for `order.placed`
  the customer is refunded). `429` and `5xx` are tried again, waiting 1, 2, 4… minutes, at most an
  hour; the messages after it wait too, keeping their order.
- **Receiving:** `GET /v2/messages?after=<cursor>&limit=50` → `{ "messages": […], "next": "…" }`,
  then `POST /v2/messages/ack { "ids": […] }`. The hub releases one message per conversation at a
  time, so MakerFoundry polls again straight away until the inbox is empty (at most 40 pages a
  round). Unreadable messages are acknowledged and logged by id only; nothing needs to be
  reachable from outside.
- **Status for the customer:** **"Sent to the shop"** while an `order.placed` is queued, and
  **"Received by the shop"** once the shop says so.
- **Expiry:** an unacknowledged message past `expires_at` is dropped, and the hub sends
  `message.expired` from `hub`: `{ "id", "type", "conversation", "to"?, "reason" }`. For an
  `order.placed` (expired, or refused by the hub), MakerFoundry refunds the customer in full (a
  ledger row) and tells them "The shop couldn't take the order, so it's refunded". A message
  that expires in MakerFoundry's own outbox before the hub takes it is treated the same way.

## Files
Art and pictures go through the hub, never by MakerFoundry's address:
- **Putting:** `POST /v2/files/put { conversation, sha256, size, kind: "art" | "mockup", to,
  type? }` → `{ url, method: "PUT", headers, expires_in }` (or `{ "ready": true }` when the hub has
  it already); MakerFoundry PUTs the bytes to `url` with exactly those headers and no service key,
  then `POST /v2/files/done { conversation, sha256 }`. An order's art is put before its
  `order.placed` is sent (the hub refuses an order naming art it doesn't have).
- **Getting** (a proof's mock-up): `POST /v2/files/get { conversation, sha256 }` → `{ url, size,
  type, expires_in }`; MakerFoundry GETs `url` with no key and checks the size and SHA-256 before
  using it.
- Caps: art 10 MB a file; pictures (previews, mock-ups) PNG or JPEG, 2 MB.

## The messages
**`catalog` (shop → MakerFoundry), v1.** Published by the shop when it opts in and whenever its
prices change; MakerFoundry keeps the latest per shop. A customer is only ever charged prices from
a catalogue the shop published. `shop.account` is the shop's Foundry Account: MakerFoundry pays
the shop through that account's payout setup, and a shop without one isn't offered to customers.
`cents` is the garment's own price per piece; each method's `price_breaks` are the decoration's
price per piece from `min` pieces up (the first starts at 1).

Shipping (the owner's decision, 2026-10-31): `flat_cents` for an order of one piece, plus the
optional `per_extra_piece_cents` for each piece after the first; with the optional
`free_over_cents` (more than 0), an order whose goods and set-up come to that or more ships free.
Both default to 0, so a catalogue with only `flat_cents` still reads. `shop.address` (optional,
the same fields as `ship_to`) is where customers pick up: sales tax on a pick-up order is worked
out from it, so a shop that offers pick-up should send it; without it MakerFoundry offers only
shipping while it collects tax.

Branding (2026-11-02, optional, never refusing a catalogue): `shop.brand` may carry
`display_name` (plain text, at most 80 characters), `intro` (plain text, at most 1,000, paragraphs
apart by a blank line, no links or HTML), `logo` (`{ size, sha256, type }`: a PNG or JPEG of at
most 2 MB put in the hub's files for the `catalog` conversation; MakerFoundry fetches it, checks it
and draws it again), `accent` (`#rrggbb`: used only where it reads at 4.5:1 on the page, light and
dark looks apart), `handle` (the shop's short address `<handle>.makerfoundry.app`: 3 to 40 of a-z,
0-9 and `-`, not starting or ending with `-`, some names reserved, first come first served) and
`website` (an `https://` address, shown as a plain link). A field that doesn't fit is left out and
the page falls back to the plain look. Example: `valid/catalog-branded.json`.
```json
{ "shop": { "name": "…", "town": "…", "ships": true, "pickup": true, "account": 4217,
            "address": { "line1": "…", "city": "…", "region": "OH", "postcode": "…", "country": "US" } },
  "products": [{ "sku": "tee-g500", "name": "Heavy cotton tee", "kind": "tee", "cents": 800,
                 "colours": ["Black", "Navy"], "sizes": ["S","M","L","XL"],
                 "methods": [{ "method": "embroidery", "placements": ["left chest","back"],
                               "price_breaks": [{ "min": 1, "cents": 2400 }, { "min": 12, "cents": 1900 }],
                               "setup_cents": 2500 }] }],
  "shipping": { "flat_cents": 899, "per_extra_piece_cents": 50, "free_over_cents": 30000 },
  "currency": "usd" }
```

**`order.placed` (MakerFoundry → shop), v1.** Sent once the payment has succeeded (Stripe's
verified webhook), never before; signed (above). Only these fields: the hub refuses others
(`not_normalized`).
```json
{ "fulfilment": "ship" | "pickup",
  "lines": [{ "sku": "tee-g500", "colour": "Navy", "sizes": { "M": 4, "L": 2 },
              "decorations": [{ "method": "embroidery", "placement": "left chest",
                                 "art": { "file": "logo.pes", "size": 48213, "sha256": "…" } }] }],
  "preview": { "size": 81234, "sha256": "…", "type": "image/png" },
  "customer": { "name": "…", "ship_to": { "name"?, "line1", "line2", "city", "region",
                                           "postcode" (or "postal"), "country" } },
  "totals": { "goods_cents": 0, "setup_cents": 0, "shipping_cents": 0, "tax_cents": 0,
              "paid_cents": 0, "fee_cents": 0, "card_fee_cents": 0, "shop_gets_cents": 0 },
  "note": "…" }
```
- Art is in the hub for the conversation (Files, above); the shop checks it by size and SHA-256.
  `preview` is optional.
- `ship_to` is there exactly when `fulfilment` is `ship`.
- The customer's email and phone are **never** sent, nor allowed in the name or the note (the
  order form refuses them; the hub refuses them too); the shop reaches the customer through the
  order's messages. Only the shipping address goes, and only when the order ships.
- StitchShop takes it into a `DecorOrder` (status `QUOTED`, its quote set from `totals`, already
  paid). The shop may turn it down (`order.cancel`) for a full refund.

**`order.status` (shop → MakerFoundry), v1.** `{ "status": "received" | "proof_sent" |
"approved" | "in_production" | "ready" | "shipped" | "collected", "tracking"?: "…", "note"?: "…" }`.
This maps StitchShop's `QUOTED → APPROVED → IN_PRODUCTION → READY → COLLECTED`: `received` is
`QUOTED` with no proof yet (the issue's "art submitted"), and `proof_sent` is `QUOTED` with a proof
out (the issue's "proof sent").

**`proof.sent` (shop → MakerFoundry), v1.** `{ "proof": 1, "mockup": { size, sha256, type },
"note": "…" }`, the mock-up in the hub for the conversation (Files, above). MakerFoundry shows the mock-up on the customer's order page (re-drawn from the
checked image, never served as sent).

**`proof.answer` (MakerFoundry → shop), v1.** `{ "proof": 1, "answer": "approved" |
"changes", "note": "…" }`. After `approved`, the shop moves the order to `APPROVED`.

**`order.cancel` (either way), v1.** `{ "by": "shop" | "customer", "why": "…" }`, `by` matching who sent it. Before
`in_production`, either side may cancel, for a full refund (MakerFoundry gives back its 10%, as
for designs). After, only the shop may, and a refund is the shop's to decide (Seller Terms).

## Money (MakerFoundry's side, as for designs)
Every charge, fee, refund and payout is a ledger row; amounts are integer cents. The customer pays
MakerFoundry by Stripe Checkout; MakerFoundry keeps 10%; the card fee comes out of the shop's share;
Stripe Connect pays the shop monthly. Shipping is the shop's, from its catalogue (above).

**Sales tax (decided 2026-10-31):** MakerFoundry takes the payment, so it's the marketplace that
collects and pays over sales tax, never the shop. Stripe Tax works it out before checkout from
where the order ships (or the shop's `address`, for pick-up), and only where MakerFoundry has
registered to collect: elsewhere it's 0. In `totals`, `tax_cents` is that tax: it's part of
`paid_cents` and **outside** the shop's share, so
`shop_gets_cents = paid_cents − tax_cents − fee_cents − card_fee_cents` (StitchHub's rule too); the 10% is of the price
before tax, the card fee of all that's paid. A shop never adds tax of its own to an order from
MakerFoundry, and reports it as a marketplace sale (the marketplace collected the tax).

## Limits
The hub's own are in `GET /v2/hello`'s `limits`, and they win. MakerFoundry's reader refuses bodies
over 256 KB; files go through the hub (art 10 MB, pictures 2 MB).

## Contract tests
`docs/contract/shop-orders/valid/` holds one valid example of every message type and version, and
`invalid/` 34 that must be refused, each named for the rule it breaks (a missing `id`, an unknown
type or version, totals that don't add up, a malformed conversation, a catalogue outside `catalog`,
an email or phone number in the note, shipping without an address, price breaks that go down, an
oversized body…). MakerFoundry's tests read both (`src/shop_orders/contract.rs`), and property tests
throw random and damaged messages at the reader (`tests/untrusted_input.rs`). They're in StitchHub's form (checked
against its `test/fixtures/sync-orders/`); `tests/stitchhub.rs` runs the client against a stand-in
hub that behaves as StitchHub describes (the handshake and `hello_first`, signatures,
`not_normalized`, files, one message per conversation at a time, duplicates, `id_reused`,
`seq_taken`).

## Open
- Shipping and sales tax (above). New Terms and Seller Terms wording for physical goods, returns
  and liability (the owner's).
- Whether the shop's catalogue can carry its own product photos (each photo through the hub, checked
  and re-encoded as MakerFoundry's photos are) or MakerFoundry draws garments in code.
- Where StitchShop's opt-in lives (a module, or a setting of the Decorating module).
