# MakerFoundry's legal pages: drafts for the owner

**Status:** 2026-10-19. Drafts, not reviewed by a lawyer, **not on the site**. Legal wording is the
owner's (docs/start-here.md). Once approved, they become `/terms`, `/privacy` and
`/seller-terms` (MF-6).

- [`terms.md`](terms.md): Terms of Service, for everyone using makerfoundry.app.
- [`privacy.md`](privacy.md): Privacy Policy.
- [`seller-terms.md`](seller-terms.md): Seller Terms, for makers who publish or sell designs.

They follow StitchFoundry's published Terms and Privacy (stitchfoundry.app/terms, /privacy):
the same business, the same Foundry Account, Rhode Island law, the same liability cap, plain
words. What they say about data matches what MakerFoundry does as built (`apps/makerfoundry`).
The copyright process matches the draft at `/dmca` (`src/moderation.rs`), and the licences the
drafts in `src/licences.rs`.

## The owner's decisions, marked [OWNER: …] in the drafts
1. **Contact address.** The drafts use `support@makerfoundry.app`, which needs Cloudflare Email
   Routing for makerfoundry.app (in the bwattendorf@gmail.com Cloudflare account) forwarding to
   the owner's Gmail. Or use support@stitchfoundry.app.
2. **The platform fee**: a percentage, a minimum, or both (Seller Terms § 4).
3. **Who may sell**: anyone with a verified Stripe account, or approved sellers first (§ 2).
4. **Sales tax on digital designs**: Stripe Tax at checkout, and whether MakerFoundry collects
   it as a marketplace facilitator (§ 5). Ideally with an accountant's word.
5. **Refunds**: the window (30 days in the draft), and whether the platform fee comes back to
   the seller (Seller Terms § 7, Terms § 8).
6. **Payout holds**: how long new sellers' payouts wait, if at all (§ 6).
7. **The small-business licence limit**: 200 items is a placeholder (Terms § 3).
8. **Minimum age**: 18, as StitchFoundry's, for accounts. Browsing and free downloads without an
   account have no age gate in the drafts.
9. **The DMCA designated agent**: registered at copyright.gov/dmca-directory, then named in the
   Terms § 7 and on `/dmca` (`MF_DMCA_AGENT`).
10. **Prices**: the lowest and highest a design may cost (Seller Terms § 3).
11. **How long reports and notices are kept** (Privacy § 2).
12. **Effective date** of each.
