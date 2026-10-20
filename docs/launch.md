# Launching MakerFoundry (MF-6)

**Status:** 2026-10-19. Everything the site needs to launch is built. The launch itself, and every
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
1. **The legal pages.** Decide each `[OWNER: …]` in `docs/legal/` (the list is in
   `docs/legal/README.md`), ideally with a lawyer's read; delete the DRAFT lines; set the
   effective dates. Ask me to make the edits if easier: I'll change only what you decide.
2. **Contact email.** Cloudflare Email Routing for makerfoundry.app (the bwattendorf@gmail.com
   Cloudflare account): `support@makerfoundry.app` forwarding to your Gmail. The legal pages and
   `/dmca` name it.
3. **DMCA agent.** Register SiteSentrix at copyright.gov/dmca-directory (a small fee), then set
   `MF_DMCA_AGENT` (lines apart by `|`: name, address, email, phone) and put the same in Terms § 7.
4. **Selling** (or launch with free designs only and open selling later):
   - In StitchFoundry's Stripe account, test mode first (`docs/deploy.md` › Stripe): turn on
     Connect (Express), make MakerFoundry's restricted key and its two webhooks, put them in
     Railway, and one test-mode sale end to end.
   - Sales tax: decide (Stripe Tax, marketplace-facilitator states), ideally with an accountant.
   - Then the live keys and live webhooks in place of the test ones.
5. **Admins.** Sign in on makerfoundry.app once; I read your account number from the database for
   `MF_ADMINS` (and anyone else who reviews reports).
6. **Launch.** Set `MF_LAUNCHED=yes`; check the deploy log has no "staying closed" line; open
   `/terms`, sign up a test account, publish a free design, and check `/robots.txt`.
7. **Tell people**: StitchFoundry's site and app can link to makerfoundry.app (MF-7 adds publishing
   from the app).
