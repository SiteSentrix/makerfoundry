# Shops that sell through MakerFoundry (MF-98): wording for the Seller Terms and the Privacy Policy

**Status:** 2026-11-02. **A draft for the owner**, not yet in the Seller Terms or the Privacy
Policy. Ordering from shops stays switched off on production until this wording (with the orders
wording still to come, `docs/shop-orders-contract.md` › Open) is approved and merged. Merging makes
a new version of each document; shops agree to the new Seller Terms version when they next confirm
their shop on their account page. Legal wording is the owner's (`docs/start-here.md`); a lawyer's
read is advised.

It describes the branded shops as built (MF-98: `apps/makerfoundry/src/shop_orders/brand.rs` and
`shops.rs`) and the fee the owner decided on 2026-11-02.

## Paid shop orders and the money rules (seller terms v3, 2026-11-05)

Paid shop orders stay switched off until shop-order chargebacks are automated (reversed, paused
and restored as for designs; today a shop-order dispute only alerts the admins). When they open,
the Seller Terms' money rules apply to shop orders too: getting paid (§ 6), chargebacks (§ 8),
what you owe us (§ 9) and holds and reserves (§ 12). The physical-goods sections (shipping and
tracking, lost or damaged packages, "item not received" disputes, returns and exchanges, defects,
product safety, items not as described) still need drafting before then.

## Seller Terms: a new section, "Shops"

> **Shops.** A decorating shop can sell its goods to customers through MakerFoundry, by taking
> orders from MakerFoundry in StitchShop and confirming the shop on its MakerFoundry account page.
>
> - **You are the seller.** The goods are yours: you make them, ship them or have them collected,
>   and you're responsible for their quality, for delivering what was ordered, and for returns and
>   for answering your customers about their orders (on the order's page on MakerFoundry).
> - **MakerFoundry takes the payment.** Customers pay MakerFoundry, which passes your share to your
>   Stripe account. **There's no monthly fee.** On each order MakerFoundry keeps **10% of the price**
>   (before sales tax), and the **card fee (2.9% + 30¢ of the whole payment)** also comes out of your
>   share. For example, on a $100 order with no sales tax you receive $86.80.
> - **Sales tax.** MakerFoundry collects and pays over the sales tax on orders placed through it,
>   as the marketplace, where it's registered to. Don't add sales tax of your own to these orders;
>   report them as marketplace sales.
> - **Refunds.** If you or the customer cancel before production, the customer gets everything
>   back, and MakerFoundry returns its 10% to you, so you're out only the card fee. After
>   production starts, a refund is yours to decide.
> - **Your shop's page.** You get a free page on MakerFoundry with your shop's name, logo, colour,
>   introduction, a link to your website and your products, at an address of your choosing on
>   makerfoundry.app (first come, first served; some names are reserved). Show only names, logos and
>   pictures that are yours or that you may use, and nothing infringing, misleading, adult or
>   unlawful. MakerFoundry may change or take down a page that breaks this, and reports of
>   infringement go through its copyright process (/dmca). An address may be taken back if it
>   misleads people or names someone else.
> - **Ending.** You can stop taking orders from MakerFoundry at any time in StitchShop; orders
>   already placed are finished (or cancelled and refunded) as above.

## Seller Terms § 4 ("Our fee"): one added sentence

> For goods sold by shops, the fee is in "Shops": 10% of the price, with the card fee also from the
> shop's share, and no monthly fee.

## Privacy Policy: lines to add

> - **Shops' pages.** What a shop puts on its page (its name, logo, colour, introduction and
>   website) is public, as its products and prices are. Its logo is fetched through StitchHub, drawn
>   again (nothing of the original file is kept) and stored with our other files (Cloudflare R2).
> - **Orders from shops** (with the orders wording still to come): what a customer's order sends
>   the shop through StitchHub, and how long it's kept (`docs/shop-orders-contract.md`).

## The owner's choices

- `[OWNER: the example amount: keep "$100 → $86.80", or none]`
- `[OWNER: whether an address can be taken back, as drafted, or only on a court or DMCA order]`
