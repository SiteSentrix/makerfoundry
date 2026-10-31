# Ordering from shops through StitchHub: the message contract (issues #2 and #1)

**Status:** 2026-10-31. **A proposal for review.** MakerFoundry's reader and writer of these messages are built (MF-60), switched off; the hub's and StitchShop's sides aren't. The owner decided (#2,
`docs/stitchshop-ordering-audit.md`): customers order physical goods from shops through
MakerFoundry, live; MakerFoundry takes the payment and keeps 10%, and the shop pays the card fee;
orders travel through **StitchHub**. This is the format the three programs would build to. The hub
owns the transport, so once agreed it moves to StitchHub's docs, with StitchShop's and
MakerFoundry's sides built against fixtures of it (contract tests on both sides).

It extends what the hub already does for the StitchFoundry inbox (stitchhub `src/inbox.mjs`): files
by short-lived signed addresses into the hub's bucket, checked by size and SHA-256; shops under
their install key. What's new: orders to **other accounts' shops that opt in**, messages both ways,
a queue for offline shops, versions, and expiry.

## Who's who
| Party | Role | Proves itself with |
|---|---|---|
| **MakerFoundry** | Takes the customer's order and payment; sends orders and the customer's answers; shows the customer the shop's status and proofs | A service key the hub admin issues (`MF_STITCHHUB_KEY`, a Railway secret) |
| **StitchHub** | Lists shops that take orders; queues and delivers messages both ways; holds art files briefly | n/a |
| **StitchShop** | Opts in; publishes its catalogue; takes orders into a decorating order; sends status and proofs | Its install key, as for backups and the inbox |

A shop opts in on StitchShop (Settings › Online orders › "Take orders from MakerFoundry"). Until it
does, the hub never lists it and refuses orders for it.

## The handshake (versions)
Every connection starts with `GET /v2/hello`, sending `X-Client: makerfoundry/0.1.0` (or
`stitchshop/…`) and the message types it understands:

```json
{ "hub": "2.0.0", "min_client": { "makerfoundry": "0.1.0", "stitchshop": "…" },
  "types": { "order.placed": [1], "order.status": [1], "proof.sent": [1], "proof.answer": [1],
             "order.cancel": [1], "catalog": [1], "message.expired": [1] },
  "deprecated": [{ "type": "…", "version": 1, "until": "2027-03-01" }] }
```

- A client older than `min_client` gets `426 Upgrade Required` on every other call, and says so
  plainly: MakerFoundry shows admins "StitchHub needs a newer MakerFoundry", and stops sending.
- `deprecated` entries are logged once a day until `until`.
- Each message type carries its own `version`. Within a version, fields are only ever added, and
  readers ignore fields they don't know. A breaking change is a new version, offered alongside the
  old until its `until` date.

## The envelope
Every message, both ways:

```json
{ "id": "0192f0c4-…",            "type": "order.status", "version": 1,
  "order": "mf-8f3a…",            "seq": 4,
  "from": "shop:<hub shop id>",   "to": "makerfoundry",
  "created_at": "2026-11-02T15:04:05Z", "expires_at": "2026-11-09T15:04:05Z",
  "body": { … } }
```

- `id`: a UUIDv7 made by the sender. **The receiver de-duplicates by `id`**: the same message
  twice is applied once. Senders retry with the same `id`.
- `order`: MakerFoundry's order reference, on every message about that order.
- `seq`: per sender per order, starting at 1. The receiver applies an order's messages in `seq`
  order, holding a later one until the earlier arrives; a gap older than 24 hours is reported to
  the admins.
- `expires_at`: when the hub gives up delivering (default 7 days for orders, 14 for proofs).

## Delivery: the queue
- **Sending:** `POST /v2/messages` with the envelope → `202 { "queued": true }`. The hub keeps
  it until the receiver acknowledges it or it expires.
- **Receiving:** `GET /v2/messages?after=<cursor>&limit=50` → the oldest unacknowledged messages
  for the caller, then `POST /v2/messages/ack { "ids": […] }`. Receivers poll (every minute) and
  after reconnecting; nothing needs to be reachable from outside, so a shop on a PC behind a router
  works.
- **Status for the customer:** MakerFoundry shows **"Sent to the shop"** while an `order.placed` is
  queued and not yet acknowledged ("the shop's program is offline; it gets your order when it's back
  on"), and **"Received by the shop"** once it is.
- **Expiry:** an unacknowledged message past `expires_at` is dropped, and the hub sends the sender
  `message.expired { "id", "type", "order" }`. For an expired `order.placed`, MakerFoundry
  refunds the customer in full (a ledger row), tells them, and marks the order "The shop didn't
  answer".

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
verified webhook), never before.
```json
{ "lines": [{ "sku": "tee-g500", "colour": "Navy", "sizes": { "M": 4, "L": 2 },
              "decorations": [{ "method": "embroidery", "placement": "left chest",
                                 "art": { "file": "logo.pes", "size": 48213, "sha256": "…" } }] }],
  "art_urls": { "<sha256>": "https://…signed, 10 minutes…" },
  "preview": { "size": 81234, "sha256": "…", "type": "image/png" },
  "customer": { "name": "…", "ship_to": { … } | null },
  "totals": { "goods_cents": 0, "setup_cents": 0, "shipping_cents": 0, "tax_cents": 0,
              "paid_cents": 0, "fee_cents": 0, "card_fee_cents": 0, "shop_gets_cents": 0 },
  "note": "…" }
```
- Art is fetched by signed address and checked by size and SHA-256 before use, as the inbox does.
  The same caps as the inbox apply: 10 MB a file, previews PNG or JPEG of 2 MB at most.
- The customer's email and phone are **never** sent; the shop reaches the customer through the
  order's messages. Only the shipping address goes, and only when the order ships.
- StitchShop takes it into a `DecorOrder` (status `QUOTED`, its quote set from `totals`, already
  paid). The shop may turn it down (`order.cancel`) for a full refund.

**`order.status` (shop → MakerFoundry), v1.** `{ "status": "received" | "proof_sent" |
"approved" | "in_production" | "ready" | "shipped" | "collected", "tracking"?: "…", "note"?: "…" }`.
This maps StitchShop's `QUOTED → APPROVED → IN_PRODUCTION → READY → COLLECTED`: `received` is
`QUOTED` with no proof yet (the issue's "art submitted"), and `proof_sent` is `QUOTED` with a proof
out (the issue's "proof sent").

**`proof.sent` (shop → MakerFoundry), v1.** `{ "proof": 1, "mockup": { size, sha256, type, url },
"note": "…" }`. MakerFoundry shows the mock-up on the customer's order page (re-drawn from the
checked image, never served as sent).

**`proof.answer` (MakerFoundry → shop), v1.** `{ "proof": 1, "answer": "approved" |
"changes", "note": "…" }`. After `approved`, the shop moves the order to `APPROVED`.

**`order.cancel` (either way), v1.** `{ "by": "shop" | "customer", "why": "…" }`. Before
`in_production`, either side may cancel, for a full refund (MakerFoundry gives back its 10%, as
for designs). After, only the shop may, and a refund is the shop's to decide (Seller Terms).

## Money (MakerFoundry's side, as for designs)
Every charge, fee, refund and payout is a ledger row; amounts are integer cents. The customer pays
MakerFoundry by Stripe Checkout; MakerFoundry keeps 10%; the card fee comes out of the shop's share;
Stripe Connect pays the shop monthly. Sales tax on physical goods depends on where they ship
(**open:** Stripe Tax, as MF-24), and so does shipping (**open:** the shop's flat price in its
catalogue, as above, or MakerFoundry's).

## Limits
Per shop: at most 200 open orders, 50 messages a minute. Per MakerFoundry: 600 messages a minute.
Bodies at most 256 KB (files go by signed address). The hub keeps delivered messages 30 days for
support, then deletes them.

## Contract tests
`docs/contract/shop-orders/valid/` holds one valid example of every message type and version, and
`invalid/` 25 that must be refused, each named for the rule it breaks (a missing `id`, an unknown
type or version, totals that don't add up, art without an address, price breaks that go down, an
oversized body…). MakerFoundry's tests read both (`src/shop_orders/contract.rs`), and property tests
throw random and damaged messages at the reader (`tests/untrusted_input.rs`). The same files are
copied into StitchHub and StitchShop, and each side's tests read and write them, so a change on one
side fails the others' tests until they agree.

## Open
- Shipping and sales tax (above). New Terms and Seller Terms wording for physical goods, returns
  and liability (the owner's).
- Whether the shop's catalogue can carry its own product photos (each photo through the hub, checked
  and re-encoded as MakerFoundry's photos are) or MakerFoundry draws garments in code.
- Where StitchShop's opt-in lives (a module, or a setting of the Decorating module).
