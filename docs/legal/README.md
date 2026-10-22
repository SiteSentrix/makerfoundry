# MakerFoundry's legal pages: drafts for the owner

**Status:** 2026-10-19. Drafts, not reviewed by a lawyer. Legal wording is the owner's
(docs/start-here.md). The site shows these files as `/terms`, `/privacy` and `/seller-terms`
(`src/legal.rs`): to admins only, marked as drafts, while any `[OWNER: …]`, "[When …]" or "DRAFT
for the owner" is left; to everyone once none is and the site has launched (`docs/launch.md`).
To approve a document, edit it here: decide each marker, delete the DRAFT line, set the date.

**Decided by the owner, 2026-10-22**, and written into the drafts: effective date October 22, 2026;
contact support@makerfoundry.app; accounts at 18 or older; the small-business licence at 200 items;
refunds within 30 days for a broken or misdescribed file; anyone with an account may sell; prices $1
to $200; no wait on a new seller's first payout; on a refund the seller gets our fee back but bears
the card fee Stripe keeps (Seller Terms § 7; built as MF-23); conversion on unless the maker turns
it off; payouts held up to 90 days after an account is closed for breaking the terms; reports and
notices kept 3 years; makes in Terms § 4. Earlier: the fee (10%, the card fee from the seller's
share, monthly payouts: Seller Terms § 4, § 6).

**Still open:** the DMCA designated agent (Terms § 7, once registered), and the DRAFT lines, deleted
once a lawyer has read the documents or the owner approves them without one. **Sales tax** waits
until paid sales open: for the free-only launch (owner's decision, 2026-10-22), Seller Terms § 5
says paid sales aren't open yet; the decision, ideally with an accountant, goes there before the
Stripe keys do (`docs/launch.md`).

- [`terms.md`](terms.md): Terms of Service, for everyone using makerfoundry.app.
- [`privacy.md`](privacy.md): Privacy Policy.
- [`seller-terms.md`](seller-terms.md): Seller Terms, for makers who publish or sell designs.

They follow StitchFoundry's published Terms and Privacy (stitchfoundry.app/terms, /privacy):
the same business, the same Foundry Account, Rhode Island law, the same liability cap, plain
words. What they say about data matches what MakerFoundry does as built (`apps/makerfoundry`).
The copyright process matches the draft at `/dmca` (`src/moderation.rs`), and the licences the
drafts in `src/licences.rs`.

## The owner's decisions, marked [OWNER: …] in the drafts
(Added since the first list: the Terms should say that makes (photos people post of what they
made from a design, MF-11) are theirs, that they give MakerFoundry permission to show them, and
that a design's maker may take a make of their design down [OWNER: wording, Terms § 4 or § 5];
Seller Terms § 9, whether converting embroidery designs to other
formats on download (MF-9) is on unless the maker turns it off, as built.)

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
