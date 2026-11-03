//! The messages MakerFoundry and shops exchange through StitchHub (MF-80, issues #1, #2 and #53):
//! `docs/shop-orders-contract.md`, in the form the hub implements (StitchHub's message, 2026-11-01),
//! with one example of each in `docs/contract/shop-orders/` that the tests here read. Everything a
//! shop sends is untrusted: every field is checked against a limit before anything is made of it.
//! Fields a reader doesn't know are ignored, so a newer sender can add fields within a version;
//! what MakerFoundry sends has only the contract's fields (the hub refuses others).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The largest message, its body included (files go through the hub, never in a message).
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;
/// The largest art file and preview a message may point to (as the hub's inbox).
pub const MAX_ART_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
/// The most pieces in one order.
pub const MAX_PIECES: u32 = 10_000;
/// The largest amount anywhere in a message: $100,000.
pub const MAX_CENTS: i64 = 10_000_000;

/// The decoration methods a shop may offer.
pub const METHODS: &[&str] = &["embroidery", "screen_print", "dtg", "heat_transfer", "vinyl"];
/// The statuses a shop reports, in their order.
pub const STATUSES: &[&str] = &[
    "received",
    "proof_sent",
    "approved",
    "in_production",
    "ready",
    "shipped",
    "collected",
];

/// MakerFoundry, as the hub names it, unless `MF_STITCHHUB_SERVICE` names it otherwise (a test
/// deploy is `service:makerfoundry-test`, with its own key and queue).
pub const ME: &str = "service:makerfoundry";

static SERVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The name this MakerFoundry sends and signs as, and receives for: `MF_STITCHHUB_SERVICE`, or
/// `service:makerfoundry`.
pub fn me() -> &'static str {
    SERVICE.get().map_or(ME, String::as_str)
}

/// Whether `name` can be a service's name: `service:` and 1 to 40 of a-z, 0-9 and `-`.
pub fn is_service_name(name: &str) -> bool {
    name.strip_prefix("service:").is_some_and(|n| {
        !n.is_empty()
            && n.len() <= 40
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// Sets the name this MakerFoundry goes by on the hub, once, at start-up.
pub fn set_me(name: &str) -> Result<(), String> {
    if !is_service_name(name) {
        return Err("MF_STITCHHUB_SERVICE must be service: and a-z, 0-9 or - (at most 40)".into());
    }
    SERVICE
        .set(name.to_string())
        .map_err(|_| "the hub name is already set".to_string())
}
/// The catalogue's conversation.
pub const CATALOG: &str = "catalog";

/// Every message's envelope. `conversation` is MakerFoundry's order reference (or `catalog`);
/// `seq` counts from 1 per sender per conversation. MakerFoundry's own messages carry no `from`
/// (the hub knows who sent them by the key); messages from the hub carry it (`shop:<id>` or
/// `hub`) and `delivery_seq`, which MakerFoundry doesn't need.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Envelope {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub conversation: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<Sig>,
}

/// An envelope's signature (`order.placed`): Ed25519, base64 (`sign.rs`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sig {
    pub v: u32,
    pub sig: String,
}

impl Envelope {
    /// Who sent it: `shop:<id>`, `hub`, or MakerFoundry.
    pub fn sender(&self) -> &str {
        self.from.as_deref().unwrap_or(me())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Shop {
    pub name: String,
    #[serde(default)]
    pub town: String,
    #[serde(default)]
    pub ships: bool,
    #[serde(default)]
    pub pickup: bool,
    /// The Foundry Account the shop belongs to: MakerFoundry pays it through that account's
    /// payout setup.
    pub account: i64,
    /// Where customers pick up (needed for sales tax on pick-up orders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    /// The shop's branding (optional; read leniently by `brand::read`, never refusing the
    /// catalogue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PriceBreak {
    pub min: u32,
    pub cents: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Method {
    pub method: String,
    pub placements: Vec<String>,
    pub price_breaks: Vec<PriceBreak>,
    #[serde(default)]
    pub setup_cents: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Product {
    pub sku: String,
    pub name: String,
    pub kind: String,
    /// The garment's own price per piece, before decoration.
    pub cents: i64,
    pub colours: Vec<String>,
    pub sizes: Vec<String>,
    pub methods: Vec<Method>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Shipping {
    /// For an order of one piece.
    pub flat_cents: i64,
    /// Added for each piece after the first.
    #[serde(default)]
    pub per_extra_piece_cents: i64,
    /// Orders whose goods and set-up come to this or more ship free; 0: never.
    #[serde(default)]
    pub free_over_cents: i64,
}

impl Shipping {
    /// What shipping `pieces` pieces costs, for goods and set-up of `subtotal`.
    pub fn cents(&self, pieces: u32, subtotal: i64) -> Option<i64> {
        if self.free_over_cents > 0 && subtotal >= self.free_over_cents {
            return Some(0);
        }
        self.per_extra_piece_cents
            .checked_mul(i64::from(pieces.saturating_sub(1)))?
            .checked_add(self.flat_cents)
    }
}

/// `catalog` v1: a shop's products and prices.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Catalog {
    pub shop: Shop,
    pub products: Vec<Product>,
    pub shipping: Shipping,
    pub currency: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FileRef {
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Decoration {
    pub method: String,
    pub placement: String,
    pub art: FileRef,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Line {
    pub sku: String,
    pub colour: String,
    pub sizes: BTreeMap<String, u32>,
    pub decorations: Vec<Decoration>,
}

/// A picture that travels through the hub (a preview, a proof's mock-up), named by its hash.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Image {
    pub size: u64,
    pub sha256: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// An address (`postal` is read as `postcode`, as the hub does).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Address {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    pub city: String,
    #[serde(default)]
    pub region: String,
    #[serde(alias = "postal")]
    pub postcode: String,
    pub country: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Customer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship_to: Option<Address>,
}

/// An order's amounts, in cents: what the customer paid, and how it's shared.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Totals {
    pub goods_cents: i64,
    pub setup_cents: i64,
    pub shipping_cents: i64,
    pub tax_cents: i64,
    pub paid_cents: i64,
    pub fee_cents: i64,
    pub card_fee_cents: i64,
    pub shop_gets_cents: i64,
}

/// Whether an optional text is blank: then it's left out of the message, never sent as `""`
/// (StitchHub refuses an empty optional text, issue #77).
#[allow(clippy::ptr_arg)]
fn blank(s: &String) -> bool {
    s.trim().is_empty()
}

/// The longest texts StitchHub takes (its `sync-orders.mjs`), so MakerFoundry never sends one it
/// refuses: notes and reasons, names and street lines, city, region (state) and tracking numbers.
pub const MAX_NOTE: usize = 500;
pub const MAX_NAME: usize = 120;
pub const MAX_CITY: usize = 80;
pub const MAX_REGION: usize = 40;
pub const MAX_POSTCODE: usize = 20;
pub const MAX_TRACKING: usize = 80;

/// `order.placed` v1: sent once the customer's payment has succeeded, signed. Its art and preview
/// are named by hash and travel through the hub's files (`hub::upload`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderPlaced {
    /// `ship` (with `customer.ship_to`) or `pickup` (without).
    pub fulfilment: String,
    pub lines: Vec<Line>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Image>,
    pub customer: Customer,
    pub totals: Totals,
    #[serde(default, skip_serializing_if = "blank")]
    pub note: String,
}

/// `order.status` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderStatus {
    pub status: String,
    #[serde(default, skip_serializing_if = "blank")]
    pub tracking: String,
    #[serde(default, skip_serializing_if = "blank")]
    pub note: String,
}

/// `proof.sent` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProofSent {
    pub proof: u32,
    pub mockup: Image,
    #[serde(default, skip_serializing_if = "blank")]
    pub note: String,
}

/// `proof.answer` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProofAnswer {
    pub proof: u32,
    pub answer: String,
    #[serde(default, skip_serializing_if = "blank")]
    pub note: String,
}

/// `order.cancel` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderCancel {
    pub by: String,
    #[serde(default)]
    pub why: String,
}

/// `message.expired` v1: the hub gave up delivering one of ours (`reason`: `expired`, or
/// `recipient_too_old`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MessageExpired {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub reason: String,
}

/// A message, its body read and checked.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Catalog(Box<Catalog>),
    OrderPlaced(Box<OrderPlaced>),
    OrderStatus(OrderStatus),
    ProofSent(ProofSent),
    ProofAnswer(ProofAnswer),
    OrderCancel(OrderCancel),
    MessageExpired(MessageExpired),
}

impl Message {
    /// Its type, as the envelope names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Message::Catalog(_) => "catalog",
            Message::OrderPlaced(_) => "order.placed",
            Message::OrderStatus(_) => "order.status",
            Message::ProofSent(_) => "proof.sent",
            Message::ProofAnswer(_) => "proof.answer",
            Message::OrderCancel(_) => "order.cancel",
            Message::MessageExpired(_) => "message.expired",
        }
    }

    fn body(&self) -> Result<Value, String> {
        let v = match self {
            Message::Catalog(m) => serde_json::to_value(m),
            Message::OrderPlaced(m) => serde_json::to_value(m),
            Message::OrderStatus(m) => serde_json::to_value(m),
            Message::ProofSent(m) => serde_json::to_value(m),
            Message::ProofAnswer(m) => serde_json::to_value(m),
            Message::OrderCancel(m) => serde_json::to_value(m),
            Message::MessageExpired(m) => serde_json::to_value(m),
        };
        v.map_err(|e| e.to_string())
    }
}

/// The types MakerFoundry reads, for the hub's handshake (`POST /v2/hello`).
pub const READS: &[(&str, u32)] = &[
    ("catalog", 1),
    ("order.status", 1),
    ("proof.sent", 1),
    ("order.cancel", 1),
    ("message.expired", 1),
];

/// The message types and versions in the contract.
pub const UNDERSTOOD: &[(&str, u32)] = &[
    ("catalog", 1),
    ("order.placed", 1),
    ("order.status", 1),
    ("proof.sent", 1),
    ("proof.answer", 1),
    ("order.cancel", 1),
    ("message.expired", 1),
];

// ---------------------------------------------------------------- checks

fn text(field: &str, s: &str, max: usize, required: bool) -> Result<(), String> {
    if required && s.trim().is_empty() {
        return Err(format!("{field} is empty"));
    }
    if s.chars().count() > max {
        return Err(format!("{field} is longer than {max} characters"));
    }
    if s.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!("{field} has control characters"));
    }
    Ok(())
}

fn address(field: &str, a: &Address) -> Result<(), String> {
    for (f, v, req, max) in [
        ("name", &a.name, false, MAX_NAME),
        ("line1", &a.line1, true, MAX_NAME),
        ("line2", &a.line2, false, MAX_NAME),
        ("city", &a.city, true, MAX_CITY),
        ("region", &a.region, false, MAX_REGION),
        ("postcode", &a.postcode, true, MAX_POSTCODE),
    ] {
        text(&format!("{field}.{f}"), v, max, req)?;
    }
    if a.country.len() != 2 || !a.country.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(format!("{field}.country is a two-letter code"));
    }
    Ok(())
}

fn cents(field: &str, c: i64) -> Result<(), String> {
    if (0..=MAX_CENTS).contains(&c) {
        Ok(())
    } else {
        Err(format!("{field} is out of range"))
    }
}

fn sha256(field: &str, s: &str) -> Result<(), String> {
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(())
    } else {
        Err(format!("{field} isn't a SHA-256"))
    }
}

/// A UUID's text form (any version), lowercase.
pub fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n)
        && s.bytes()
            .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, with or without fractions of a second (`.123`).
pub fn is_time(s: &str) -> bool {
    let b = s.as_bytes();
    let head = b.get(..19).is_some_and(|h| {
        h.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        })
    });
    let tail = match b.get(19..) {
        Some([b'Z']) => true,
        Some([b'.', rest @ .., b'Z']) => (1..=9).contains(&rest.len()) && rest.iter().all(u8::is_ascii_digit),
        _ => false,
    };
    head && tail
}

/// A shop, as the hub names it: `shop:<hub shop id>`.
pub fn is_shop(s: &str) -> bool {
    s.strip_prefix("shop:").is_some_and(|id| {
        !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    })
}

/// A recipient MakerFoundry may be: its own service name (`makerfoundry` read too, for the default).
fn is_me(s: &str) -> bool {
    s == me() || (me() == ME && s == "makerfoundry")
}

/// A conversation: 1 to 120 of letters, digits and `. _ : -`.
pub fn is_conversation(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

/// An order reference MakerFoundry made: `mf-` and up to 40 letters and digits.
pub fn is_order_ref(s: &str) -> bool {
    s.strip_prefix("mf-")
        .is_some_and(|r| !r.is_empty() && r.len() <= 40 && r.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn image(field: &str, i: &Image) -> Result<(), String> {
    if !(1..=MAX_PREVIEW_BYTES).contains(&i.size) {
        return Err(format!("{field} is empty or larger than 2 MB"));
    }
    sha256(&format!("{field}.sha256"), &i.sha256)?;
    if i.kind != "image/png" && i.kind != "image/jpeg" {
        return Err(format!("{field} isn't a PNG or JPEG"));
    }
    Ok(())
}

/// Whether `text` looks like it holds an email address or a phone number: the hub refuses an
/// order whose note does (`personal_contact`), because the shop reaches the customer through the
/// order's messages, never directly.
pub fn has_contact(text: &str) -> bool {
    let email = text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.split_once('@').is_some_and(|(user, host)| {
            !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
        })
    });
    // seven or more digits in a run that only spaces, dots, dashes, brackets or + break up
    let mut digits = 0;
    let mut phone = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits += 1;
            if digits >= 7 {
                phone = true;
            }
        } else if !" .-()+/".contains(c) {
            digits = 0;
        }
    }
    email || phone
}

fn check_catalog(c: &Catalog) -> Result<(), String> {
    text("shop.name", &c.shop.name, 100, true)?;
    text("shop.town", &c.shop.town, 100, false)?;
    if c.shop.account <= 0 {
        return Err("shop.account is missing".into());
    }
    if c.currency != "usd" {
        return Err("only usd is taken".into());
    }
    cents("shipping.flat_cents", c.shipping.flat_cents)?;
    cents("shipping.per_extra_piece_cents", c.shipping.per_extra_piece_cents)?;
    cents("shipping.free_over_cents", c.shipping.free_over_cents)?;
    if let Some(a) = &c.shop.address {
        address("shop.address", a)?;
    }
    if c.products.is_empty() || c.products.len() > 200 {
        return Err("a catalogue has 1 to 200 products".into());
    }
    let mut skus = std::collections::BTreeSet::new();
    for p in &c.products {
        let ok = !p.sku.is_empty()
            && p.sku.len() <= 40
            && p.sku.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
        if !ok {
            return Err("a product's sku is malformed".into());
        }
        if !skus.insert(p.sku.as_str()) {
            return Err(format!("sku {} appears twice", p.sku));
        }
        text("product.name", &p.name, 100, true)?;
        text("product.kind", &p.kind, 30, true)?;
        cents("product.cents", p.cents)?;
        if p.colours.is_empty() || p.colours.len() > 40 || p.sizes.is_empty() || p.sizes.len() > 20 {
            return Err(format!("{}: 1 to 40 colours and 1 to 20 sizes", p.sku));
        }
        for c in &p.colours {
            text("colour", c, 40, true)?;
        }
        for s in &p.sizes {
            text("size", s, 10, true)?;
        }
        if p.methods.is_empty() || p.methods.len() > METHODS.len() {
            return Err(format!("{}: 1 to {} methods", p.sku, METHODS.len()));
        }
        for m in &p.methods {
            if !METHODS.contains(&m.method.as_str()) {
                return Err(format!("{}: unknown method {}", p.sku, m.method));
            }
            if m.placements.is_empty() || m.placements.len() > 10 {
                return Err(format!("{}: 1 to 10 placements", p.sku));
            }
            for pl in &m.placements {
                text("placement", pl, 40, true)?;
            }
            cents("setup_cents", m.setup_cents)?;
            if m.price_breaks.is_empty() || m.price_breaks.len() > 10 {
                return Err(format!("{}: 1 to 10 price breaks", p.sku));
            }
            if m.price_breaks.first().map(|b| b.min) != Some(1) {
                return Err(format!("{}: the first price break starts at 1", p.sku));
            }
            for w in m.price_breaks.windows(2) {
                if let [a, b] = w
                    && b.min <= a.min
                {
                    return Err(format!("{}: price breaks must go up", p.sku));
                }
            }
            for b in &m.price_breaks {
                cents("price_breaks.cents", b.cents)?;
                if b.min > MAX_PIECES {
                    return Err(format!("{}: a price break is too large", p.sku));
                }
            }
        }
    }
    Ok(())
}

/// Checks the totals add up: what was paid is the parts, and the shop gets it less the fees.
pub fn check_totals(t: &Totals) -> Result<(), String> {
    for (f, c) in [
        ("goods_cents", t.goods_cents),
        ("setup_cents", t.setup_cents),
        ("shipping_cents", t.shipping_cents),
        ("tax_cents", t.tax_cents),
        ("paid_cents", t.paid_cents),
        ("fee_cents", t.fee_cents),
        ("card_fee_cents", t.card_fee_cents),
        ("shop_gets_cents", t.shop_gets_cents),
    ] {
        cents(f, c)?;
    }
    if t.goods_cents + t.setup_cents + t.shipping_cents + t.tax_cents != t.paid_cents {
        return Err("the totals don't add up to what was paid".into());
    }
    // (sales tax is collected by MakerFoundry, the marketplace: never the shop's)
    if t.paid_cents - t.tax_cents - t.fee_cents - t.card_fee_cents != t.shop_gets_cents {
        return Err("the shop's share doesn't add up".into());
    }
    Ok(())
}

fn check_placed(o: &OrderPlaced) -> Result<(), String> {
    if o.lines.is_empty() || o.lines.len() > 30 {
        return Err("an order has 1 to 30 lines".into());
    }
    let mut pieces: u32 = 0;
    for l in &o.lines {
        text("line.sku", &l.sku, 40, true)?;
        text("line.colour", &l.colour, 40, true)?;
        if l.sizes.is_empty() || l.sizes.len() > 20 {
            return Err("a line has 1 to 20 sizes".into());
        }
        for (size, n) in &l.sizes {
            text("size", size, 10, true)?;
            if *n == 0 {
                return Err("a size has no pieces".into());
            }
            pieces = pieces.saturating_add(*n);
        }
        if l.decorations.is_empty() || l.decorations.len() > 5 {
            return Err("a line has 1 to 5 decorations".into());
        }
        for d in &l.decorations {
            if !METHODS.contains(&d.method.as_str()) {
                return Err(format!("unknown method {}", d.method));
            }
            text("placement", &d.placement, 40, true)?;
            let ok = !d.art.file.is_empty()
                && d.art.file.len() <= 120
                && d.art
                    .file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b" ._()+-".contains(&b));
            if !ok {
                return Err("an art file's name is malformed".into());
            }
            if !(1..=MAX_ART_BYTES).contains(&d.art.size) {
                return Err("an art file is empty or larger than 10 MB".into());
            }
            sha256("art.sha256", &d.art.sha256)?;
        }
    }
    if pieces > MAX_PIECES {
        return Err(format!("an order has at most {MAX_PIECES} pieces"));
    }
    if let Some(p) = &o.preview {
        image("preview", p)?;
    }
    text("customer.name", &o.customer.name, 100, true)?;
    match (o.fulfilment.as_str(), &o.customer.ship_to) {
        ("ship", Some(a)) => address("ship_to", a)?,
        ("ship", None) => return Err("a shipped order needs its address".into()),
        ("pickup", None) => {}
        ("pickup", Some(_)) => return Err("a pick-up order has no address".into()),
        _ => return Err("fulfilment is ship or pickup".into()),
    }
    check_totals(&o.totals)?;
    text("note", &o.note, MAX_NOTE, false)?;
    if has_contact(&o.note) || has_contact(&o.customer.name) {
        return Err("the note has an email address or phone number".into());
    }
    Ok(())
}

fn check(m: &Message) -> Result<(), String> {
    match m {
        Message::Catalog(c) => check_catalog(c),
        Message::OrderPlaced(o) => check_placed(o),
        Message::OrderStatus(s) => {
            if !STATUSES.contains(&s.status.as_str()) {
                return Err(format!("unknown status {}", s.status));
            }
            text("tracking", &s.tracking, MAX_TRACKING, false)?;
            text("note", &s.note, MAX_NOTE, false)
        }
        Message::ProofSent(p) => {
            if !(1..=100).contains(&p.proof) {
                return Err("proof is numbered 1 to 100".into());
            }
            image("mockup", &p.mockup)?;
            text("note", &p.note, MAX_NOTE, false)
        }
        Message::ProofAnswer(a) => {
            if !(1..=100).contains(&a.proof) {
                return Err("proof is numbered 1 to 100".into());
            }
            match a.answer.as_str() {
                "approved" => {}
                "changes" if !a.note.trim().is_empty() => {}
                "changes" => return Err("asking for changes needs a note".into()),
                _ => return Err("an answer is approved or changes".into()),
            }
            text("note", &a.note, MAX_NOTE, false)?;
            if has_contact(&a.note) {
                return Err("the note has an email address or phone number".into());
            }
            Ok(())
        }
        Message::OrderCancel(c) => {
            if c.by != "shop" && c.by != "customer" {
                return Err("by is shop or customer".into());
            }
            // (StitchHub needs a reason: MakerFoundry gives one when the customer didn't)
            text("why", &c.why, MAX_NOTE, true)?;
            if has_contact(&c.why) {
                return Err("the reason has an email address or phone number".into());
            }
            Ok(())
        }
        Message::MessageExpired(e) => {
            if !is_uuid(&e.id) {
                return Err("id isn't a UUID".into());
            }
            if !UNDERSTOOD.iter().any(|(k, _)| *k == e.kind) {
                return Err("unknown type".into());
            }
            if !is_conversation(&e.conversation) {
                return Err("conversation is malformed".into());
            }
            if e.to.as_deref().is_some_and(|t| !is_shop(t)) {
                return Err("to isn't a shop".into());
            }
            text("reason", &e.reason, 40, true)
        }
    }
}

/// Reads and checks a message.
pub fn parse(bytes: &[u8]) -> Result<(Envelope, Message), String> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("the message is larger than 256 KB".into());
    }
    let env: Envelope = serde_json::from_slice(bytes).map_err(|e| format!("not a message: {e}"))?;
    if !is_uuid(&env.id) {
        return Err("id isn't a UUID".into());
    }
    // to a shop from MakerFoundry, or to MakerFoundry from a shop or the hub
    let ours = env.from.as_deref().is_none_or(is_me);
    let from_ok = match env.from.as_deref() {
        None => true,
        Some(f) => is_me(f) || f == "hub" || is_shop(f),
    };
    let to_ok = if ours { is_shop(&env.to) } else { is_me(&env.to) };
    if !from_ok || !to_ok {
        return Err("from or to isn't a party".into());
    }
    if env.from.as_deref() == Some("hub") && env.kind != "message.expired" {
        return Err("the hub sends only message.expired".into());
    }
    if !is_time(&env.created_at) || env.expires_at.as_deref().is_some_and(|e| !is_time(e)) {
        return Err("created_at or expires_at isn't a UTC time".into());
    }
    if env.expires_at.as_deref().is_some_and(|e| e <= env.created_at.as_str()) {
        return Err("it expires before it was made".into());
    }
    if !UNDERSTOOD.contains(&(env.kind.as_str(), env.version)) {
        return Err(format!("{} v{} isn't understood", env.kind, env.version));
    }
    if env.seq < 1 || !is_conversation(&env.conversation) {
        return Err("conversation or seq is malformed".into());
    }
    let about_order = !matches!(env.kind.as_str(), "catalog" | "message.expired");
    if env.kind == "catalog" && env.conversation != CATALOG {
        return Err("a catalogue's conversation is catalog".into());
    }
    if about_order && !is_order_ref(&env.conversation) {
        return Err("an order message's conversation is one of MakerFoundry's orders".into());
    }
    let body = env.body.clone();
    let read = |e: serde_json::Error| format!("{}: {e}", env.kind);
    let msg = match env.kind.as_str() {
        "catalog" => Message::Catalog(serde_json::from_value(body).map_err(read)?),
        "order.placed" => Message::OrderPlaced(serde_json::from_value(body).map_err(read)?),
        "order.status" => Message::OrderStatus(serde_json::from_value(body).map_err(read)?),
        "proof.sent" => Message::ProofSent(serde_json::from_value(body).map_err(read)?),
        "proof.answer" => Message::ProofAnswer(serde_json::from_value(body).map_err(read)?),
        "order.cancel" => Message::OrderCancel(serde_json::from_value(body).map_err(read)?),
        _ => Message::MessageExpired(serde_json::from_value(body).map_err(read)?),
    };
    check(&msg).map_err(|why| format!("{}: {why}", env.kind))?;
    // a cancel says who cancelled, and only they can say so
    if let Message::OrderCancel(c) = &msg
        && c.by != if ours { "customer" } else { "shop" }
    {
        return Err("order.cancel: by isn't its sender".into());
    }
    Ok((env, msg))
}

// ---------------------------------------------------------------- making messages

/// A new UUIDv7 (RFC 9562 § 5.7): the time in milliseconds, then randomness.
pub fn new_id() -> Result<String, String> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
    let ms = ms.to_be_bytes();
    // the low 48 bits of the time
    b[..6].copy_from_slice(ms.get(10..16).ok_or("time")?);
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ` (civil from days, H. Hinnant's algorithm).
pub fn utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The time now, as messages carry it.
pub fn now_utc() -> String {
    utc(now())
}

/// A message from MakerFoundry in `conversation`, checked as a receiver would check it (it's
/// stamped and, for `order.placed`, signed again when it's sent: `hub::send`).
pub fn make(message: &Message, conversation: &str, seq: u64, to: &str, days: i64) -> Result<Envelope, String> {
    let t = now();
    let env = Envelope {
        id: new_id()?,
        kind: message.kind().to_string(),
        version: 1,
        conversation: conversation.to_string(),
        seq,
        from: None,
        to: to.to_string(),
        created_at: utc(t),
        expires_at: Some(utc(t.saturating_add(days.saturating_mul(86_400)))),
        body: message.body()?,
        sig: None,
    };
    let bytes = serde_json::to_vec(&env).map_err(|e| e.to_string())?;
    parse(&bytes)?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_optional_texts_are_left_out_never_sent_empty() {
        // (issue #77: StitchHub refuses an empty optional text)
        let answer = ProofAnswer {
            proof: 1,
            answer: "approved".into(),
            note: "  ".into(),
        };
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({"proof": 1, "answer": "approved"})
        );
        let status = OrderStatus {
            status: "received".into(),
            tracking: String::new(),
            note: String::new(),
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({"status": "received"})
        );
        let with = ProofAnswer {
            proof: 1,
            answer: "changes".into(),
            note: "Smaller".into(),
        };
        assert_eq!(serde_json::to_value(&with).unwrap()["note"], "Smaller");
        // and a cancel needs its reason
        let cancel = Message::OrderCancel(OrderCancel {
            by: "customer".into(),
            why: String::new(),
        });
        assert!(make(&cancel, "mf-8f3a1", 1, "shop:mainst", 7).is_err());
    }

    #[test]
    fn a_service_name_is_service_and_plain_letters() {
        for good in ["service:makerfoundry", "service:makerfoundry-test", "service:a1"] {
            assert!(is_service_name(good), "{good}");
        }
        for bad in [
            "makerfoundry",
            "service:",
            "service:Maker",
            "service:a b",
            "service:a_b",
            "shop:x",
            &format!("service:{}", "a".repeat(41)),
        ] {
            assert!(!is_service_name(bad), "{bad}");
        }
    }

    #[test]
    fn shipping_is_flat_plus_each_extra_piece_and_free_over_a_total() {
        let s = Shipping {
            flat_cents: 899,
            per_extra_piece_cents: 50,
            free_over_cents: 30_000,
        };
        assert_eq!(s.cents(1, 2_000), Some(899));
        assert_eq!(s.cents(12, 22_900), Some(899 + 11 * 50));
        assert_eq!(s.cents(20, 29_999), Some(899 + 19 * 50));
        assert_eq!(s.cents(20, 30_000), Some(0));
        // the old form: one flat price, never free
        let flat = Shipping {
            flat_cents: 899,
            per_extra_piece_cents: 0,
            free_over_cents: 0,
        };
        assert_eq!(flat.cents(500, MAX_CENTS), Some(899));
        assert_eq!(
            Shipping {
                flat_cents: 0,
                per_extra_piece_cents: i64::MAX,
                free_over_cents: 0
            }
            .cents(3, 0),
            None
        );
    }

    #[test]
    fn a_v1_catalogue_without_the_new_shipping_fields_still_reads() {
        let mut v: serde_json::Value = serde_json::from_slice(
            &fixtures("valid")
                .into_iter()
                .find(|(n, _)| n == "catalog.json")
                .unwrap()
                .1,
        )
        .unwrap();
        v["body"]["shipping"] = serde_json::json!({"flat_cents": 899});
        v["body"]["shop"].as_object_mut().unwrap().remove("address");
        let (_, m) = parse(v.to_string().as_bytes()).unwrap();
        let Message::Catalog(c) = m else { panic!("a catalogue") };
        assert_eq!(c.shipping.per_extra_piece_cents, 0);
        assert_eq!(c.shipping.free_over_cents, 0);
        assert!(c.shop.address.is_none());
    }

    fn fixtures(kind: &str) -> Vec<(String, Vec<u8>)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/contract/shop-orders")
            .join(kind);
        let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&p).unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn every_valid_fixture_reads_and_round_trips() {
        let valid = fixtures("valid");
        let kinds: std::collections::BTreeSet<&str> = valid
            .iter()
            .map(|(_, b)| {
                let (env, msg) = parse(b).unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(env.kind, msg.kind());
                // written again, it reads the same
                let again = Envelope {
                    body: msg.body().unwrap(),
                    ..env.clone()
                };
                assert_eq!(parse(&serde_json::to_vec(&again).unwrap()).unwrap().1, msg);
                msg.kind()
            })
            .collect();
        let all: std::collections::BTreeSet<&str> = UNDERSTOOD.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, all, "one valid example of every type");
    }

    #[test]
    fn every_invalid_fixture_is_refused() {
        let invalid = fixtures("invalid");
        assert!(invalid.len() >= 10);
        for (name, b) in invalid {
            assert!(parse(&b).is_err(), "{name} should be refused");
        }
    }

    #[test]
    fn unknown_fields_are_ignored_within_a_version() {
        let (_, b) = fixtures("valid")
            .into_iter()
            .find(|(n, _)| n.starts_with("order.status"))
            .unwrap();
        let mut v: Value = serde_json::from_slice(&b).unwrap();
        v["body"]["colour_of_the_sky"] = "blue".into();
        v["added_later"] = 1.into();
        assert!(parse(&serde_json::to_vec(&v).unwrap()).is_ok());
    }

    #[test]
    fn ids_and_times_are_well_formed() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert!(is_uuid(&a) && a != b);
        assert_eq!(&a[14..15], "7", "version 7");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"), "the RFC variant");
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_793_454_245), "2026-10-31T13:44:05Z");
        assert!(is_time(&utc(now())));
    }

    #[test]
    fn made_messages_are_checked_like_received_ones() {
        let status = Message::OrderStatus(OrderStatus {
            status: "ready".into(),
            tracking: String::new(),
            note: String::new(),
        });
        let env = make(&status, "mf-abc123", 3, "shop:mainst", 7).unwrap();
        assert_eq!(
            (env.from.as_deref(), env.seq),
            (None, 3),
            "no from: the hub knows us by our key"
        );
        assert_eq!(env.sender(), me());
        assert!(env.expires_at.is_some() && env.sig.is_none());
        assert!(
            make(&status, CATALOG, 3, "shop:mainst", 7).is_err(),
            "an order message is in its order's conversation"
        );
        assert!(
            make(&status, "mf-abc123", 0, "shop:mainst", 7).is_err(),
            "seq counts from 1"
        );
        let bad = Message::OrderStatus(OrderStatus {
            status: "lost".into(),
            tracking: String::new(),
            note: String::new(),
        });
        assert!(make(&bad, "mf-abc123", 1, "shop:mainst", 7).is_err());
        assert!(make(&status, "mf-abc123", 1, "nobody", 7).is_err());
        assert!(
            make(&status, "mf-abc123", 1, "service:makerfoundry", 7).is_err(),
            "only to shops"
        );
    }

    #[test]
    fn times_may_carry_fractions_of_a_second() {
        assert!(is_time("2026-11-02T15:04:05Z"));
        assert!(is_time("2026-11-02T15:04:05.1Z"));
        assert!(is_time("2026-11-02T15:04:05.123456789Z"));
        for bad in [
            "2026-11-02T15:04:05",
            "2026-11-02T15:04:05.Z",
            "2026-11-02T15:04:05.1234567890Z",
            "2026-11-02 15:04:05Z",
            "2026-11-02T15:04:05+00:00",
        ] {
            assert!(!is_time(bad), "{bad}");
        }
    }

    #[test]
    fn contact_details_in_a_note_are_found() {
        for yes in [
            "write me at pat@example.com",
            "call 401 555 0100 after five",
            "(401) 555-0100",
            "+1.401.555.0100",
            "pat.lee@mail.example.org.",
        ] {
            assert!(has_contact(yes), "{yes}");
        }
        for no in [
            "Please match the navy of the school shirts.",
            "12 navy, 6 black, size 2XL",
            "Logo 3.5 inches wide @ left chest",
            "order 123456 please",
            "",
        ] {
            assert!(!has_contact(no), "{no}");
        }
    }

    #[test]
    fn totals_must_add_up() {
        let t = Totals {
            goods_cents: 10_000,
            setup_cents: 2_500,
            shipping_cents: 899,
            tax_cents: 0,
            paid_cents: 13_399,
            fee_cents: 1_340,
            card_fee_cents: 419,
            shop_gets_cents: 11_640,
        };
        assert!(check_totals(&t).is_ok());
        assert!(
            check_totals(&Totals {
                shop_gets_cents: 11_641,
                ..t.clone()
            })
            .is_err()
        );
        assert!(
            check_totals(&Totals {
                paid_cents: 13_398,
                ..t.clone()
            })
            .is_err()
        );
        assert!(
            check_totals(&Totals {
                tax_cents: -1,
                paid_cents: 13_398,
                ..t
            })
            .is_err()
        );
    }
}
