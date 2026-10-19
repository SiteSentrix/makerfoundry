# What other design marketplaces charge sellers (October 2026)

Research for the owner's platform-fee decision (MF-4). Figures are from the sites' own pages
where found, otherwise from 2025–2026 third-party guides (cited); check a site's current terms
before quoting it. "Keeps" is what the seller is left with on a **$5 design**, a typical price for
an embroidery or SVG file, before income tax.

| Site | What it charges sellers | Seller keeps on $5 | Notes |
|---|---|---|---|
| **Thingiverse** | Nothing: free files only, no selling | n/a | Bought by MyMiniFactory in Feb 2026; "monetisation pathways" promised, not yet announced |
| **Printables** (Prusa) | No paid sales; rewards are Prusa store credit; paid "clubs" | n/a | |
| **MakerWorld** (Bambu) | 10% on paid memberships + PayPal fees | ≈ $4.20 (membership) | Points/royalties scheme for free models |
| **Cults3D** | 20% of the price (Cults 15% + bank fees 5%), no subscription | $4.00 | Official creator page |
| **Thangs** | 30% under $5, 20% $5–10, 15% $10+; + Stripe 2.9% + 30¢ | ≈ $3.55 | Official FAQ |
| **MyMiniFactory** | ~10–15% + a monthly creator plan | ≈ $4.25, less the plan | Figures vary by source |
| **Etsy** | $0.20 listing + 6.5% + 3% + 25¢ processing (US); Offsite Ads 12–15% on ad-driven sales | ≈ $4.08 | The main place embroidery/SVG files sell today |
| **Creative Fabrica** | Official terms: designer gets 50% of single sales (marketing says up to 75% on self-referred); subscription downloads paid from a points pool, reportedly cents each | $2.50 (≈ $3.75 self-referred) | Juried; may include your work in its subscription |
| **Design Bundles** | Designer 75% direct, 50% via an affiliate | $3.75 (or $2.50) | $5 minimum price |
| **Gumroad** | 10% + 50¢ (30% via its Discover marketplace) | ≈ $4.00 or less | Your own store |
| **Payhip** | 5% free plan (2% / 0% on $29 / $99 plans) + processing | ≈ $4.30 | Your own store |
| **Ko-fi** | 5% free plan (0% with Gold) + processing | ≈ $4.30 | Your own store |
| **EmbroideryDesigns.com** | "Competitive royalties", not published | ? | Embroidery-specific; ask them |

## What it would cost MakerFoundry (Stripe Connect)

- Card processing: 2.9% + 30¢ a payment ($0.45 on $5: 9% of a small sale).
- **Express** accounts: $2 a month for each seller paid that month, plus 0.25% + 25¢ a payout.
  A seller with one $5 sale in a month could cost more than a 10% fee brings in, so payouts
  should be batched (monthly, or above a minimum such as $25).
- **Standard** accounts: no Connect fees for the platform; the seller has a full Stripe account
  and pays its card fee directly. Less to build and cheaper for MakerFoundry, but sellers deal
  with Stripe more directly, and who counts as the seller for sales tax needs an accountant's
  view either way.

## Where a MakerFoundry fee would sit

| MakerFoundry fee | Seller keeps on $5 (Stripe's 45¢ passed to the seller) | Compared |
|---|---|---|
| 10% | ≈ $4.05 (81%) | ≈ Etsy, better than Cults, Thangs, Creative Fabrica, Design Bundles |
| 15% | ≈ $3.80 (76%) | Below Etsy, still above Thangs, CF, DB |
| 20% | ≈ $3.55 (71%) | Same as Cults/Thangs at this price |

Sources: cults3d.com/en/upload; thangs.com help centre ("What fees does Thangs charge?");
creativefabrica.com Designers Terms & Conditions V1.3; designbundles.net help centre ("How much do
you make on Design Bundles?"); craftybase.com and sellerfeecalc.com (Etsy 2026); 3druck.com
(MakerWorld memberships); pulse2.com and 3dadept.com (Thingiverse acquisition); payhip.com,
sellfy.com and schoolmaker.com (Gumroad, Payhip, Ko-fi); drop-desk.com and
isvpaymentintegration.com (Stripe Connect pricing).
