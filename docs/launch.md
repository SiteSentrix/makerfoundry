# Launching MakerFoundry (MF-6)

**Status:** 2026-10-22. The legal pages are approved and the launch switch can open (free designs
only; the DMCA agent the week after). Everything the site needs to launch is built. The launch itself, and every
step below, is the owner's (docs/start-here.md: legal wording, money, anything public).

## What the launch switch does
Set `MF_LAUNCHED=yes` in Railway's variables (service `makerfoundry`). On the next deploy:

- **It checks the legal pages first.** If any of `docs/legal/terms.md`, `privacy.md` or
  `seller-terms.md` still has an `[OWNER: …]` marker, a "[When …]" note or the "DRAFT for the
  owner" line, the site stays closed and the deploy log says which document has how many left.
- `/terms`, `/privacy` and `/seller-terms` become public (before: admins only, marked as drafts).
- Signing up asks people to confirm they're 18 or older and agree to the Terms and the Privacy
  Policy; publishing a design and setting up selling ask them to agree to the Seller Terms. Each
  agreement is kept with the document's version (`mf_agreements`), so a later change of wording
  is a new version.
- Publishing opens to everyone (unless `MF_PUBLISHERS` is set, which still limits it).
- The home page swaps "Coming soon" for **Browse designs** and **Share your designs**.
- `robots.txt` lets search engines in (not account, admin or download pages) and `/sitemap.xml`
  lists the published designs and makers. Before the launch `robots.txt` keeps them out.

Turning it off again (`MF_LAUNCHED` removed) closes publishing to new makers and hides the legal
pages; published designs stay up.

## The owner's steps, in order
1. **The legal pages.** ~~Decide each `[OWNER: …]` in `docs/legal/`, delete the DRAFT lines, set
   the effective dates.~~ **Done 2026-10-22** (approved by the owner; a lawyer's read later).
2. **Contact email.** Cloudflare Email Routing for makerfoundry.app (the bwattendorf@gmail.com
   Cloudflare account): `support@makerfoundry.app` forwarding to your Gmail. The legal pages and
   `/dmca` name it. **Done 2026-10-22** (MX at route1-3.mx.cloudflare.net, SPF includes
   Cloudflare's).
3. **DMCA agent.** Register SiteSentrix at copyright.gov/dmca-directory ($6, valid three years;
   makerfoundry.app as an alternate name), then set `MF_DMCA_AGENT` (lines apart by `|`: name,
   address, email, phone): `/dmca` shows it, and Terms § 7 points there. The owner chose a soft
   open first (2026-10-22), registering the week after: until then `/dmca` asks for the form, and
   the DMCA's safe harbour (17 U.S.C. § 512(c)(2)) isn't in place, so act on reports promptly.
4. **Selling** (or launch with free designs only and open selling later):
   - In StitchFoundry's Stripe account, test mode first (`docs/deploy.md` › Stripe): turn on
     Connect (Express), make MakerFoundry's restricted key and its two webhooks, put them in
     Railway, and one test-mode sale end to end.
   - Sales tax: built (MF-24, `docs/deploy.md` › Sales tax: Stripe Tax, off until
     `MF_STRIPE_TAX=yes`). Decide (marketplace-facilitator states), ideally with an accountant,
     and put it in Seller Terms § 5 in place of "Paid sales aren't open yet" (the free-only launch
     wording, 2026-10-22) **before** the Stripe keys go in: the new version asks sellers to agree
     again.
   - Then the live keys and live webhooks in place of the test ones.
5. **Admins.** Sign in on makerfoundry.app once; I read your account number from the database for
   `MF_ADMINS` (and anyone else who reviews reports).
6. **Launch.** Set `MF_LAUNCHED=yes`; check the deploy log has no "staying closed" line; open
   `/terms`, sign up a test account, publish a free design, and check `/robots.txt`.
7. **Tell people**: StitchFoundry's site and app can link to makerfoundry.app (MF-7 adds publishing
   from the app).
