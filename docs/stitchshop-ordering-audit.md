# MakerFoundry → StitchShop ordering: audit (issue #2, step 1)

**Status:** 2026-10-31. **Report only.** This is the issue's approval gate: nothing below is built,
and the next steps wait for the owner. Sources: this repository at `main`; Stitchwright/stitchbench
at `47df015` (StitchShop is the same code base) and Stitchwright/stitchhub at `2d0b883`, both read,
nothing copied. Issue #1 (the StitchHub sync client) is covered at the end, because #2's handoff
would ride on it.

## How MakerFoundry and StitchShop connect today
| Link | Today |
|---|---|
| **Shared sign-in** | Yes. Both use the Foundry Account (cloud.stitchfoundry.app). MakerFoundry has done since MF-1; StitchBench/StitchShop since FA-5, behind `FOUNDRY_SIGNIN`. The same person is the same account `id` in both. |
| **An API between them** | None. MakerFoundry's API (`docs/api.md`) takes designs from the apps and serves design files and 3D models. StitchShop's public API (`/api/public/v1/{slug}/…`, the shop's `sbk_` key) answers ticket status, hours, classes and drop-off booking. Neither knows the other exists. |
| **Webhooks** | None between them. MakerFoundry's only webhooks are Stripe's. |
| **An order schema** | Not shared. MakerFoundry sells **digital** designs (an `mf_orders` row is a Stripe Checkout for a download). StitchShop has `DecorOrder` (garment lines with sizes, a decoration per placement, a mock-up, a quote in cents, status `QUOTED → APPROVED → IN_PRODUCTION → READY → COLLECTED`, or `CANCELLED`/`TURNED_DOWN`) and its own online shop (`src/lib/shop/online-shop.ts`, retail goods, Stripe). |
| **Through StitchHub** | Not for MakerFoundry. The hub has an **inbox** (stitchhub `src/inbox.mjs`, StitchShop `src/lib/decor-inbox.ts`): **StitchFoundry** sends an embroidery file, with garment and placement details and a preview, to a shop **on the same Foundry Account**, and the shop takes it into a `DecorOrder`. The bytes go through the hub's bucket by signed addresses; StitchShop checks size and SHA-256. |
| **Proofs** | Two separate ones. MakerFoundry's approval links (MF-25, `src/proofs.rs`) let a shop send a customer a design to approve, inside MakerFoundry. StitchShop's quote link (its answer posted to `/api/e/{token}`) lets a customer approve a decorating order's quote and mock-up, inside StitchShop. They don't talk to each other. |

**In short:** they share an identity and nothing else. The closest existing path is the hub inbox,
built for a designer sending their **own** file to their **own** shop.

## What's missing for issue #2's flow, end to end
1. **Orders addressed to someone else's shop.** The inbox only lists the sender's own shops
   (`GET /v1/inbox/shops`). A customer ordering from Main Street Threadworks isn't on the shop's
   account. Needs a hub change (a shop that chooses to accept orders, found by slug, with its own
   limits and abuse controls) or a direct MakerFoundry → StitchShop endpoint. **Decision 1.**
2. **A physical-goods storefront in MakerFoundry.** MakerFoundry has no products, garments, sizes,
   decoration methods, quantity-break pricing or shipping. This is a new product line for a
   marketplace of digital designs. **Decision 2** (money, anything public: the owner's).
3. **Mock-ups.** StitchShop stores a mock-up a person made. A live mock-up (a design placed on a
   garment picture) would be new: server-drawn in Rust (our rule: no JS frameworks), with original
   garment art (`ASSETS.md`). The issue's "AI mockups capped/canned" adds cost and a service.
4. **Status names.** The issue's "art submitted" and "proof sent" aren't StitchShop statuses. They
   map to `QUOTED` (art in, no quote link yet) and `QUOTED` with a quote link sent. A shared mapping
   (or new statuses there) belongs in the contract.
5. **The proof round trip.** Today a StitchShop quote link is approved on StitchShop's own page.
   Showing it "back in the customer view" on MakerFoundry needs StitchShop to report status
   changes back: a callback or polling, signed, through the hub or directly.
6. **Payment.** Who charges the customer: MakerFoundry (the broker, as for designs), or the shop
   through StitchShop's own Stripe? **Decision 3.**
7. **No demo in MakerFoundry (owner, 2026-10-31).** MakerFoundry is the live system and gets no
   demo mode, demo store or demo data. The demo for issue #2 lives in StitchShop's demo
   (`DEMO_MODE=on`, stitchbench `docs/demo.md`), which already fences messages, uploads and rate.
   Anything MakerFoundry builds for ordering is the real feature, in Stripe test mode only until
   the owner switches it to live keys, like selling designs. So the issue's MakerFoundry-side demo
   items (test-mode demo store, nightly reset of demo orders and its guard test) don't apply here.
   rows).
8. **Contract tests on both sides.** Neither repo has a fixture of the other's messages.

## Recommendation
Build it in this order, each step its own approval:
1. **Agree the message contract first** (a short doc in StitchHub, the hub owning the transport):
   order placed, art and preview, status changed, proof sent and approved, cancelled; message ids,
   ordering per order, expiry. That's also issue #1's ground.
2. **The hub change:** shops opt in to receive orders from other accounts (Decision 1).
3. **MakerFoundry's ordering, as a real feature** (if Decision 2 says yes): switched off until the
   owner turns it on, tested against a stand-in StitchShop/hub, as the other outside services are.
4. **StitchShop's side:** take an order message into a `DecorOrder`, and report status back.

## The owner's decisions before anything is built
- **Decision 1:** hub-routed orders (the hub accepts orders for opted-in shops: one transport, works
  when shops are offline, matches #1) or direct MakerFoundry → StitchShop calls (simpler, but
  every shop must be online and reachable). *Recommended: hub-routed.*
- **Decision 2:** does MakerFoundry offer customers ordering physical goods from shops at all?
  (No demo either way: owner, 2026-10-31.)
  This changes the Terms, the Seller Terms and who's liable for physical goods.
- **Decision 3:** who takes the payment for a real order.
- Original placeholder art for Main Street Threadworks: drawn in code here, or supplied.

## Issue #1: the StitchHub sync client
Blocked, as the issue says. The hub design it depends on (versioned APIs, a capability handshake,
an offline message queue: "Phase 2", stitchhub#19) isn't in StitchHub's code or docs yet: neither
`src/` nor `docs/` has a handshake, versions or an outbound queue. Also, its premise ("MakerFoundry
sends orders and proofs to shops through StitchHub") isn't true today: MakerFoundry sends nothing
to shops. So #1 starts after #2's Decision 1 and the hub's Phase 2. Building a client to a protocol
that isn't designed yet would mean guessing the wire format.
