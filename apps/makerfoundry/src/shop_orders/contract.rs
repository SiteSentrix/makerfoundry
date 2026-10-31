//! The messages MakerFoundry and shops exchange through StitchHub (MF-60, issues #1 and #2):
//! `docs/shop-orders-contract.md`, with one example of each in `docs/contract/shop-orders/` that
//! the tests here read (StitchHub and StitchShop test against the same files). Everything a shop
//! sends is untrusted: every field is checked against a limit before anything is made of it.
//! Fields a reader doesn't know are ignored, so a newer sender can add fields within a version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The largest message, its body included (files go by signed address, never in a message).
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

/// Every message's envelope.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Envelope {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(default)]
    pub seq: u64,
    pub from: String,
    pub to: String,
    pub created_at: String,
    pub expires_at: String,
    pub body: Value,
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
    pub flat_cents: i64,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Image {
    pub size: u64,
    pub sha256: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Address {
    pub line1: String,
    #[serde(default)]
    pub line2: String,
    pub city: String,
    pub region: String,
    pub postcode: String,
    pub country: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Customer {
    pub name: String,
    #[serde(default)]
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

/// `order.placed` v1: sent once the customer's payment has succeeded.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderPlaced {
    pub lines: Vec<Line>,
    pub art_urls: BTreeMap<String, String>,
    #[serde(default)]
    pub preview: Option<Image>,
    pub customer: Customer,
    pub totals: Totals,
    #[serde(default)]
    pub note: String,
}

/// `order.status` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderStatus {
    pub status: String,
    #[serde(default)]
    pub tracking: String,
    #[serde(default)]
    pub note: String,
}

/// `proof.sent` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProofSent {
    pub proof: u32,
    pub mockup: Image,
    #[serde(default)]
    pub note: String,
}

/// `proof.answer` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProofAnswer {
    pub proof: u32,
    pub answer: String,
    #[serde(default)]
    pub note: String,
}

/// `order.cancel` v1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OrderCancel {
    pub by: String,
    #[serde(default)]
    pub why: String,
}

/// `message.expired` v1: the hub gave up delivering one of ours.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MessageExpired {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub order: Option<String>,
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

/// The message types and versions MakerFoundry understands (for the hub's handshake).
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

fn https(field: &str, s: &str) -> Result<(), String> {
    if s.len() <= 2048 && s.starts_with("https://") && !s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Ok(())
    } else {
        Err(format!("{field} isn't an https address"))
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

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn is_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// A party: `makerfoundry`, or `shop:<hub shop id>`.
fn is_party(s: &str) -> bool {
    s == "makerfoundry"
        || s.strip_prefix("shop:").is_some_and(|id| {
            !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// An order reference MakerFoundry made: `mf-` and up to 40 letters and digits.
pub fn is_order_ref(s: &str) -> bool {
    s.strip_prefix("mf-")
        .is_some_and(|r| !r.is_empty() && r.len() <= 40 && r.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn image(field: &str, i: &Image, needs_url: bool) -> Result<(), String> {
    if !(1..=MAX_PREVIEW_BYTES).contains(&i.size) {
        return Err(format!("{field} is empty or larger than 2 MB"));
    }
    sha256(&format!("{field}.sha256"), &i.sha256)?;
    if i.kind != "image/png" && i.kind != "image/jpeg" {
        return Err(format!("{field} isn't a PNG or JPEG"));
    }
    match (&i.url, needs_url) {
        (Some(u), _) => https(&format!("{field}.url"), u),
        (None, true) => Err(format!("{field} has no address")),
        (None, false) => Ok(()),
    }
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
    if t.paid_cents - t.fee_cents - t.card_fee_cents != t.shop_gets_cents {
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
            match o.art_urls.get(&d.art.sha256) {
                Some(u) => https("art_urls", u)?,
                None => return Err("an art file has no address".into()),
            }
        }
    }
    if pieces > MAX_PIECES {
        return Err(format!("an order has at most {MAX_PIECES} pieces"));
    }
    if let Some(p) = &o.preview {
        image("preview", p, false)?;
    }
    text("customer.name", &o.customer.name, 100, true)?;
    if let Some(a) = &o.customer.ship_to {
        for (f, v, req) in [
            ("line1", &a.line1, true),
            ("line2", &a.line2, false),
            ("city", &a.city, true),
            ("region", &a.region, true),
            ("postcode", &a.postcode, true),
        ] {
            text(f, v, 100, req)?;
        }
        if a.country.len() != 2 || !a.country.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err("country is a two-letter code".into());
        }
    }
    check_totals(&o.totals)?;
    text("note", &o.note, 1000, false)
}

fn check(m: &Message) -> Result<(), String> {
    match m {
        Message::Catalog(c) => check_catalog(c),
        Message::OrderPlaced(o) => check_placed(o),
        Message::OrderStatus(s) => {
            if !STATUSES.contains(&s.status.as_str()) {
                return Err(format!("unknown status {}", s.status));
            }
            text("tracking", &s.tracking, 100, false)?;
            text("note", &s.note, 1000, false)
        }
        Message::ProofSent(p) => {
            if !(1..=100).contains(&p.proof) {
                return Err("proof is numbered 1 to 100".into());
            }
            image("mockup", &p.mockup, true)?;
            text("note", &p.note, 1000, false)
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
            text("note", &a.note, 1000, false)
        }
        Message::OrderCancel(c) => {
            if c.by != "shop" && c.by != "customer" {
                return Err("by is shop or customer".into());
            }
            text("why", &c.why, 1000, false)
        }
        Message::MessageExpired(e) => {
            if !is_uuid(&e.id) {
                return Err("id isn't a UUID".into());
            }
            if !UNDERSTOOD.iter().any(|(k, _)| *k == e.kind) {
                return Err("unknown type".into());
            }
            match &e.order {
                Some(o) if !is_order_ref(o) => Err("order isn't one of ours".into()),
                _ => Ok(()),
            }
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
    if !is_party(&env.from) || !is_party(&env.to) {
        return Err("from or to isn't a party".into());
    }
    if !is_time(&env.created_at) || !is_time(&env.expires_at) {
        return Err("created_at or expires_at isn't a UTC time".into());
    }
    if env.expires_at <= env.created_at {
        return Err("it expires before it was made".into());
    }
    if !UNDERSTOOD.contains(&(env.kind.as_str(), env.version)) {
        return Err(format!("{} v{} isn't understood", env.kind, env.version));
    }
    let about_order = !matches!(env.kind.as_str(), "catalog" | "message.expired");
    match (&env.order, about_order) {
        (Some(o), true) if is_order_ref(o) && env.seq >= 1 => {}
        (_, true) => return Err("an order message needs its order and seq".into()),
        (Some(_), false) => return Err("this type isn't about one order".into()),
        (None, false) => {}
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

/// A message from MakerFoundry, checked as a receiver would check it.
pub fn make(message: &Message, order: Option<&str>, seq: u64, to: &str, days: i64) -> Result<Envelope, String> {
    let t = now();
    let env = Envelope {
        id: new_id()?,
        kind: message.kind().to_string(),
        version: 1,
        order: order.map(str::to_string),
        seq,
        from: "makerfoundry".into(),
        to: to.to_string(),
        created_at: utc(t),
        expires_at: utc(t.saturating_add(days.saturating_mul(86_400))),
        body: message.body()?,
    };
    let bytes = serde_json::to_vec(&env).map_err(|e| e.to_string())?;
    parse(&bytes)?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let env = make(&status, Some("mf-abc123"), 3, "shop:mainst", 7).unwrap();
        assert_eq!((env.from.as_str(), env.seq), ("makerfoundry", 3));
        assert!(
            make(&status, None, 3, "shop:mainst", 7).is_err(),
            "an order message needs its order"
        );
        let bad = Message::OrderStatus(OrderStatus {
            status: "lost".into(),
            tracking: String::new(),
            note: String::new(),
        });
        assert!(make(&bad, Some("mf-abc123"), 1, "shop:mainst", 7).is_err());
        assert!(make(&status, Some("mf-abc123"), 1, "nobody", 7).is_err());
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
