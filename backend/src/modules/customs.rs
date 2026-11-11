//! Bangladesh customs: duty calculation and the Bill of Entry.
//!
//! Assessable value (AV) = CIF value in BDT at the customs rate + landing charge (1% by default).
//!   CD  = AV x CD%                      Customs Duty
//!   RD  = AV x RD%                      Regulatory Duty
//!   SD  = (AV + CD + RD) x SD%          Supplementary Duty
//!   VAT = (AV + CD + RD + SD) x VAT%    Value Added Tax (recoverable as input VAT)
//!   AIT = AV x AIT%                     Advance Income Tax (adjustable against income tax)
//!   AT  = (AV + CD + RD + SD) x AT%     Advance Tax (recoverable through the VAT return)
//! All rates are stored per HS code and can be changed whenever the tariff schedule changes.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, pct, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, finance}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Rates { #[serde(default)] pub cd: Decimal, #[serde(default)] pub rd: Decimal, #[serde(default)] pub sd: Decimal, #[serde(default)] pub vat: Decimal, #[serde(default)] pub ait: Decimal, #[serde(default)] pub at: Decimal }

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Duty { pub assessable_value: Decimal, pub cd: Decimal, pub rd: Decimal, pub sd: Decimal, pub vat: Decimal, pub ait: Decimal, pub at: Decimal }
impl Duty {
    /// Everything payable to customs for this line.
    pub fn total(&self) -> Decimal { self.cd + self.rd + self.sd + self.vat + self.ait + self.at }
    /// The part that becomes product cost (not recoverable).
    pub fn capitalised(&self) -> Decimal { self.cd + self.rd + self.sd }
}

/// Compute import duties for one tariff line from its CIF value in BDT.
pub fn compute_duty(cif_bdt: Decimal, landing_charge_pct: Decimal, r: &Rates) -> Duty {
    let av = r2(cif_bdt * (Decimal::ONE_HUNDRED + landing_charge_pct) / Decimal::ONE_HUNDRED);
    let cd = pct(av, r.cd);
    let rd = pct(av, r.rd);
    let sd = pct(av + cd + rd, r.sd);
    let vat_base = av + cd + rd + sd;
    Duty { assessable_value: av, cd, rd, sd, vat: pct(vat_base, r.vat), ait: pct(av, r.ait), at: pct(vat_base, r.at) }
}

async fn rates_of(conn: &mut PgConnection, company: Uuid, hs: Uuid) -> AppResult<(String, Rates)> {
    let r: (String, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT code, cd_rate, rd_rate, sd_rate, vat_rate, ait_rate, at_rate FROM hs_codes WHERE id = $1 AND company_id = $2").bind(hs).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field("hs_code_id", "HS code does not exist"))?;
    Ok((r.0, Rates { cd: r.1, rd: r.2, sd: r.3, vat: r.4, ait: r.5, at: r.6 }))
}

#[derive(Deserialize)]
struct CalcIn { cif_value: Decimal, fx_rate: Option<Decimal>, hs_code_id: Option<Uuid>, rates: Option<Rates>, landing_charge_pct: Option<Decimal> }
/// What-if duty calculator for quotations and planning. Nothing is saved.
async fn calculator(State(st): State<AppState>, user: AuthUser, Json(b): Json<CalcIn>) -> AppResult<Json<Value>> {
    if b.cif_value <= Decimal::ZERO { return Err(AppError::field("cif_value", "Must be more than zero")); }
    let mut conn = st.db.acquire().await?;
    let (hs, rates) = match (b.hs_code_id, b.rates) { (Some(h), _) => { let (c, r) = rates_of(&mut conn, user.company_id, h).await?; (Some(c), r) }, (None, Some(r)) => (None, r), (None, None) => return Err(AppError::field("hs_code_id", "Choose an HS code or enter the rates")) };
    let landing = match b.landing_charge_pct { Some(l) => l, None => db::setting_dec(&mut conn, user.company_id, "customs.landing_charge_pct", Decimal::ONE).await? };
    let rate = b.fx_rate.unwrap_or(Decimal::ONE);
    if rate <= Decimal::ZERO { return Err(AppError::field("fx_rate", "Must be more than zero")); }
    let cif_bdt = r2(b.cif_value * rate);
    let d = compute_duty(cif_bdt, landing, &rates);
    Ok(Json(json!({"data": {"hs_code": hs, "cif_bdt": cif_bdt, "landing_charge_pct": landing, "rates": rates, "duty": d, "total_payable": d.total(), "added_to_cost": d.capitalised(), "recoverable": d.vat + d.at + d.ait,
        "effective_cost_rate_pct": if cif_bdt.is_zero() { Decimal::ZERO } else { db::r4((d.assessable_value - cif_bdt + d.capitalised()) / cif_bdt * Decimal::ONE_HUNDRED) }}})))
}

#[derive(Deserialize)]
struct BeLine { shipment_line_id: Uuid, hs_code_id: Option<Uuid>, cif_value: Option<Decimal> }
#[derive(Deserialize)]
struct BeIn { shipment_id: Uuid, be_no: String, be_date: Option<NaiveDate>, customs_station: Option<String>, cnf_agent: Option<String>, customs_rate: Decimal, freight: Option<Decimal>, insurance: Option<Decimal>, #[serde(default)] other_fees: Decimal, #[serde(default)] lines: Vec<BeLine> }

pub const BE_DOC: &str = "SELECT (to_jsonb(b) || jsonb_build_object('shipment_no', h.shipment_no, 'currency', h.currency, 'supplier_name', (SELECT name FROM suppliers s WHERE s.id = h.supplier_id),
  'capitalised', b.cd + b.rd + b.sd + b.other_fees, 'recoverable', b.vat + b.at + b.ait,
  'lines', (SELECT coalesce(jsonb_agg(to_jsonb(l) || jsonb_build_object('sku', p.sku, 'product_name', p.name) ORDER BY p.sku), '[]') FROM bill_of_entry_lines l JOIN products p ON p.id = l.product_id WHERE l.be_id = b.id)))::text
  FROM bills_of_entry b JOIN shipments h ON h.id = b.shipment_id WHERE b.id = $1 AND b.company_id = $2";

async fn create_be(State(st): State<AppState>, user: AuthUser, Json(b): Json<BeIn>) -> AppResult<Created> {
    if b.be_no.trim().is_empty() { return Err(AppError::field("be_no", "Enter the Bill of Entry number (C-number)")); }
    if b.customs_rate <= Decimal::ZERO { return Err(AppError::field("customs_rate", "Enter the customs exchange rate")); }
    if b.other_fees < Decimal::ZERO { return Err(AppError::field("other_fees", "Must be zero or more")); }
    let mut tx = st.db.begin().await?;
    let (status, no, goods, s_freight, s_ins): (String, String, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT status, shipment_no, goods_value, freight, insurance FROM shipments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(b.shipment_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("shipment_id", "Shipment does not exist"))?;
    if !["in_transit", "arrived"].contains(&status.as_str()) { return Err(AppError::rule(format!("{no} is {status}; a Bill of Entry is lodged for goods in transit or arrived"))); }
    let landing = db::setting_dec(&mut tx, user.company_id, "customs.landing_charge_pct", Decimal::ONE).await?;
    let ins_pct = db::setting_dec(&mut tx, user.company_id, "customs.default_insurance_pct", Decimal::ONE).await?;
    // Customs value needs freight and insurance even when the supplier's invoice is FOB.
    let freight = b.freight.unwrap_or(s_freight);
    let insurance = match b.insurance { Some(i) => i, None if s_ins > Decimal::ZERO => s_ins, None => pct(goods + freight, ins_pct) };
    if freight < Decimal::ZERO || insurance < Decimal::ZERO { return Err(AppError::field("freight", "Must be zero or more")); }
    let date = match b.be_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let id: Uuid = sqlx::query_scalar("INSERT INTO bills_of_entry (company_id, shipment_id, be_no, be_date, customs_station, cnf_agent, customs_rate, landing_charge_pct, other_fees, created_by) VALUES ($1,$2,$3,$4,coalesce($5,'Custom House, Chattogram'),$6,$7,$8,$9,$10) RETURNING id")
        .bind(user.company_id).bind(b.shipment_id).bind(b.be_no.trim()).bind(date).bind(&b.customs_station).bind(&b.cnf_agent).bind(b.customs_rate).bind(landing).bind(r2(b.other_fees)).bind(user.user_id)
        .fetch_one(&mut *tx).await.map_err(|e| match AppError::from(e) { AppError::Conflict(_) => AppError::Conflict("This shipment already has a Bill of Entry, or the number is already used".into()), o => o })?;
    let lines: Vec<(Uuid, Uuid, Option<Uuid>, Decimal, String)> = sqlx::query_as("SELECT l.id, l.product_id, l.hs_code_id, l.line_value, p.sku FROM shipment_lines l JOIN products p ON p.id = l.product_id WHERE l.shipment_id = $1 ORDER BY l.id").bind(b.shipment_id).fetch_all(&mut *tx).await?;
    let extra = freight + insurance;
    let (mut t, mut cif_total, mut extra_left) = (Duty { assessable_value: Decimal::ZERO, cd: Decimal::ZERO, rd: Decimal::ZERO, sd: Decimal::ZERO, vat: Decimal::ZERO, ait: Decimal::ZERO, at: Decimal::ZERO }, Decimal::ZERO, extra);
    for (i, (line, product, default_hs, value, sku)) in lines.iter().enumerate() {
        let over = b.lines.iter().find(|x| x.shipment_line_id == *line);
        let hs = over.and_then(|o| o.hs_code_id).or(*default_hs).ok_or_else(|| AppError::rule(format!("Product {sku} has no HS code. Set it on the product, or choose one for this line.")))?;
        let (code, rates) = rates_of(&mut tx, user.company_id, hs).await?;
        // Freight and insurance are spread over the lines by value; the last line takes the rounding remainder.
        let share = if i + 1 == lines.len() { extra_left } else if goods.is_zero() { Decimal::ZERO } else { r2(extra * *value / goods) };
        extra_left -= share;
        let cif = match over.and_then(|o| o.cif_value) { Some(c) if c > Decimal::ZERO => r2(c), Some(_) => return Err(AppError::field("lines", "CIF value must be more than zero")), None => *value + share };
        let d = compute_duty(r2(cif * b.customs_rate), landing, &rates);
        sqlx::query("INSERT INTO bill_of_entry_lines (be_id, shipment_line_id, product_id, hs_code, cif_value, assessable_value, cd_rate, rd_rate, sd_rate, vat_rate, ait_rate, at_rate, cd, rd, sd, vat, ait, at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)")
            .bind(id).bind(line).bind(product).bind(&code).bind(cif).bind(d.assessable_value).bind(rates.cd).bind(rates.rd).bind(rates.sd).bind(rates.vat).bind(rates.ait).bind(rates.at).bind(d.cd).bind(d.rd).bind(d.sd).bind(d.vat).bind(d.ait).bind(d.at).execute(&mut *tx).await?;
        t.assessable_value += d.assessable_value; t.cd += d.cd; t.rd += d.rd; t.sd += d.sd; t.vat += d.vat; t.ait += d.ait; t.at += d.at; cif_total += cif;
    }
    sqlx::query("UPDATE bills_of_entry SET cif_value = $2, assessable_value = $3, cd = $4, rd = $5, sd = $6, vat = $7, ait = $8, at = $9, total_payable = $10 WHERE id = $1")
        .bind(id).bind(cif_total).bind(t.assessable_value).bind(t.cd).bind(t.rd).bind(t.sd).bind(t.vat).bind(t.ait).bind(t.at).bind(t.total() + r2(b.other_fees)).execute(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET status = 'customs' WHERE id = $1 AND status = 'arrived'").bind(b.shipment_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE import_documents SET status = 'submitted', doc_no = $2, doc_date = $3, updated_at = now() WHERE shipment_id = $1 AND doc_type = 'BE'").bind(b.shipment_id).bind(b.be_no.trim()).bind(date).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "bills_of_entry", id, json!({"be_no": b.be_no, "total_payable": t.total() + b.other_fees})).await?;
    let doc: String = sqlx::query_scalar(BE_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn be_head(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(String, String, Uuid)> {
    sqlx::query_as("SELECT status, be_no, shipment_id FROM bills_of_entry WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("Bill of Entry"))
}

async fn delete_be(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let mut tx = st.db.begin().await?;
    let (status, no, shipment) = be_head(&mut tx, user.company_id, id).await?;
    if status != "assessed" { return Err(AppError::rule(format!("Bill of Entry {no} is {status}; only an unpaid assessment can be removed"))); }
    sqlx::query("DELETE FROM bills_of_entry WHERE id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET status = 'arrived' WHERE id = $1 AND status = 'customs'").bind(shipment).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "delete", "bills_of_entry", id, json!({"be_no": no})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

#[derive(Deserialize)]
struct BePay { bank_account_id: Uuid, paid_date: Option<NaiveDate> }
/// Pay the assessed duties. Non-recoverable duties join the product cost; VAT, AT and AIT become tax assets.
async fn pay_be(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<BePay>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, shipment) = be_head(&mut tx, user.company_id, id).await?;
    if status != "assessed" { return Err(AppError::rule(format!("Bill of Entry {no} is already {status}"))); }
    let bank = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bank.currency != "BDT" { return Err(AppError::field("bank_account_id", "Duties are paid from a BDT account")); }
    let (cd, rd, sd, vat, ait, at, fees, total): (Decimal, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT cd, rd, sd, vat, ait, at, other_fees, total_payable FROM bills_of_entry WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    let date = match b.paid_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    finance::ensure_funds(&mut tx, user.company_id, &bank, total).await?;
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "bill_of_entry", source_id: Some(id), memo: &format!("Customs duty, Bill of Entry {no}"), user: user.user_id }, vec![
        JL::dr(Acc::Key("git"), cd + rd + sd + fees).tag("shipment", shipment).memo("CD, RD, SD and customs fees"),
        JL::dr(Acc::Key("vat_input"), vat).memo("Import-stage VAT"),
        JL::dr(Acc::Key("at_paid"), at).memo("Advance Tax"),
        JL::dr(Acc::Key("ait_paid"), ait).memo("Advance Income Tax"),
        JL::cr(Acc::Id(bank.gl), total),
    ]).await?;
    sqlx::query("UPDATE bills_of_entry SET status = 'paid', bank_account_id = $2, paid_date = $3, journal_entry_id = $4 WHERE id = $1").bind(id).bind(b.bank_account_id).bind(date).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "pay", "bills_of_entry", id, json!({"total": total})).await?;
    let doc: String = sqlx::query_scalar(BE_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct BeRelease { released_date: Option<NaiveDate> }
async fn release_be(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<BeRelease>>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, shipment) = be_head(&mut tx, user.company_id, id).await?;
    if status != "paid" { return Err(AppError::rule(format!("Bill of Entry {no} is {status}; pay the duties before release"))); }
    let arrived: Option<NaiveDate> = sqlx::query_scalar("SELECT arrived_at FROM shipments WHERE id = $1").bind(shipment).fetch_one(&mut *tx).await?;
    if arrived.is_none() { return Err(AppError::rule("Mark the shipment as arrived before releasing it from customs")); }
    let date = match body.and_then(|b| b.0.released_date) { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    sqlx::query("UPDATE bills_of_entry SET status = 'released', released_date = $2 WHERE id = $1").bind(id).bind(date).execute(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET status = 'cleared' WHERE id = $1 AND status IN ('arrived','customs')").bind(shipment).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO tracking_events (shipment_id, event_time, event_type, description, source, created_by) VALUES ($1,$2,'customs_released',$3,'system',$4)").bind(shipment).bind(Utc::now()).bind(format!("Released by customs, Bill of Entry {no}")).bind(user.user_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "release", "bills_of_entry", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(BE_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_be(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("b.id, b.be_no, b.be_date, b.customs_station, b.cnf_agent, b.customs_rate, b.assessable_value, b.cd, b.rd, b.sd, b.vat, b.ait, b.at, b.other_fees, b.total_payable, b.status, b.paid_date, b.released_date, b.shipment_id, h.shipment_no, (SELECT name FROM suppliers s WHERE s.id = h.supplier_id) AS supplier_name FROM bills_of_entry b JOIN shipments h ON h.id = b.shipment_id WHERE b.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "b.status", "");
    db::filter_eq(&mut qb, &q, "shipment_id", "b.shipment_id", "::uuid");
    db::filter_dates(&mut qb, &q, "b.be_date");
    db::filter_search(&mut qb, &q, &["b.be_no", "h.shipment_no"]);
    db::page_json(&st.db, qb, "be_date DESC", &q).await
}
async fn get_be(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(BE_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("Bill of Entry"))?))
}

pub fn api() -> Api {
    Api::new("Import: customs")
        .post("/customs/duty-calculator", "import.read", "Estimate duties for a CIF value and HS code (nothing is saved)", calculator).body(r#"{"cif_value":46200,"fx_rate":122.5,"hs_code_id":"<uuid>"}"#)
        .get("/bills-of-entry", "import.read", "Bills of Entry", list_be).query("page,per_page,q,status,shipment_id,from,to")
        .post("/bills-of-entry", "import.write", "Lodge a Bill of Entry: duties are computed per HS code", create_be)
        .body(r#"{"shipment_id":"<uuid>","be_no":"C-1284551","be_date":"2026-12-25","cnf_agent":"Karnaphuli C&F Ltd","customs_rate":122.5,"other_fees":3500}"#)
        .get("/bills-of-entry/:id", "import.read", "One Bill of Entry with duty per line", get_be)
        .delete("/bills-of-entry/:id", "import.write", "Remove an unpaid assessment to lodge it again", delete_be)
        .post("/bills-of-entry/:id/pay", "import.pay", "Pay assessed duties from a bank account", pay_be).body(r#"{"bank_account_id":"<uuid>","paid_date":"2026-12-26"}"#)
        .post("/bills-of-entry/:id/release", "import.write", "Customs released the goods", release_be).body(r#"{"released_date":"2026-12-27"}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(s: &str) -> Decimal { Decimal::from_str(s).unwrap() }

    #[test]
    fn duty_follows_bangladesh_cascade() {
        // CIF Tk 1,000,000; landing 1% => AV 1,010,000
        let r = Rates { cd: d("25"), rd: d("3"), sd: d("20"), vat: d("15"), ait: d("5"), at: d("5") };
        let x = compute_duty(d("1000000"), d("1"), &r);
        assert_eq!(x.assessable_value, d("1010000.00"));
        assert_eq!(x.cd, d("252500.00"));
        assert_eq!(x.rd, d("30300.00"));
        assert_eq!(x.sd, d("258560.00"));          // (1,010,000 + 252,500 + 30,300) x 20%
        assert_eq!(x.vat, d("232704.00"));         // 1,551,360 x 15%
        assert_eq!(x.ait, d("50500.00"));
        assert_eq!(x.at, d("77568.00"));           // 1,551,360 x 5%
        assert_eq!(x.total(), d("902132.00"));
        assert_eq!(x.capitalised(), d("541360.00"));
    }

    #[test]
    fn zero_rated_line_only_pays_what_applies() {
        let r = Rates { cd: d("0"), rd: d("0"), sd: d("0"), vat: d("15"), ait: d("5"), at: d("0") };
        let x = compute_duty(d("123456.78"), d("1"), &r);
        assert_eq!(x.assessable_value, d("124691.35"));
        assert_eq!(x.cd + x.rd + x.sd, d("0"));
        assert_eq!(x.vat, d("18703.70"));
        assert_eq!(x.ait, d("6234.57"));
    }

    #[test]
    fn rounding_is_half_away_from_zero_to_paisa() {
        let r = Rates { cd: d("1"), ..Default::default() };
        assert_eq!(compute_duty(d("50.50"), d("0"), &r).cd, d("0.51")); // 0.505 -> 0.51
    }
}
