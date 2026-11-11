//! Background alerts: raise notifications for things that need attention soon.
//! Every alert carries a dedupe key, so running the job again never repeats a notification.
use crate::{error::AppResult, modules::notifications::{self, Note}, state::AppState};
use chrono::NaiveDate;
use sqlx::PgConnection;
use uuid::Uuid;

pub async fn run_alerts(st: &AppState) -> AppResult<u64> {
    let companies: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM companies WHERE is_active").fetch_all(&st.db).await?;
    let mut n = 0;
    for c in companies {
        let mut tx = st.db.begin().await?;
        n += company_alerts(&mut tx, c).await?;
        tx.commit().await?;
    }
    Ok(n)
}

async fn count_notes(conn: &mut PgConnection, c: Uuid) -> i64 { sqlx::query_scalar("SELECT count(*) FROM notifications WHERE company_id = $1").bind(c).fetch_one(&mut *conn).await.unwrap_or(0) }

async fn company_alerts(conn: &mut PgConnection, c: Uuid) -> AppResult<u64> {
    let before = count_notes(conn, c).await;
    let today: NaiveDate = crate::db::today(conn, c).await?;

    let lcs: Vec<(Uuid, String, NaiveDate)> = sqlx::query_as("SELECT id, lc_no, expiry_date FROM letters_of_credit WHERE company_id = $1 AND status = 'opened' AND expiry_date IS NOT NULL AND expiry_date <= $2 + 14").bind(c).bind(today).fetch_all(&mut *conn).await?;
    for (id, no, exp) in lcs {
        let left = (exp - today).num_days();
        notifications::notify_roles(conn, c, &["import_officer", "accountant"], &Note { severity: if left <= 3 { "critical" } else { "warning" }, title: &format!("LC {no} {}", if left < 0 { "has expired".to_string() } else { format!("expires in {left} days") }), title_bn: Some("এলসি মেয়াদ শেষ হচ্ছে"), body: "Amend or close the LC before it lapses.", entity: Some("letters_of_credit"), entity_id: Some(id), dedupe: Some(format!("lc-expiry:{id}:{}", if left < 0 { "x".into() } else { (left / 3).to_string() })) }).await?;
    }

    let boxes: Vec<(Uuid, String, String, NaiveDate)> = sqlx::query_as("SELECT h.id, h.shipment_no, c.container_no, c.free_until FROM containers c JOIN shipments h ON h.id = c.shipment_id WHERE h.company_id = $1 AND c.free_until IS NOT NULL AND c.delivered_at IS NULL AND c.free_until <= $2 + 2").bind(c).bind(today).fetch_all(&mut *conn).await?;
    for (id, shp, cno, free) in boxes {
        let left = (free - today).num_days();
        notifications::notify_roles(conn, c, &["import_officer", "warehouse"], &Note { severity: if left < 0 { "critical" } else { "warning" }, title: &format!("Demurrage risk: {cno} ({shp})"), title_bn: Some("ডেমারেজ ঝুঁকি"), body: &if left < 0 { format!("Free time ended {} days ago. Charges are running.", -left) } else { format!("Free time ends in {left} days. Arrange delivery.") }, entity: Some("shipments"), entity_id: Some(id), dedupe: Some(format!("demurrage:{cno}:{today}")) }).await?;
    }

    let missing: Vec<(Uuid, String, i64)> = sqlx::query_as("SELECT h.id, h.shipment_no, count(*) FROM shipments h JOIN import_documents d ON d.shipment_id = h.id AND d.is_required AND d.status IN ('pending','requested') WHERE h.company_id = $1 AND h.status IN ('booked','in_transit') AND h.eta IS NOT NULL AND h.eta <= $2 + 7 GROUP BY h.id, h.shipment_no").bind(c).bind(today).fetch_all(&mut *conn).await?;
    for (id, no, k) in missing {
        notifications::notify_roles(conn, c, &["import_officer"], &Note { severity: "warning", title: &format!("{no} arrives within 7 days with {k} document(s) missing"), title_bn: Some("কাগজপত্র অনুপস্থিত"), body: "Chase the supplier before the vessel arrives.", entity: Some("shipments"), entity_id: Some(id), dedupe: Some(format!("docs-missing:{id}:{today}")) }).await?;
    }

    let overdue: Vec<(Uuid, String, rust_decimal::Decimal)> = sqlx::query_as("SELECT c.id, c.name, sum(i.total - i.paid_amount - i.returned_amount) FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.company_id = $1 AND i.status IN ('open','partial') AND i.due_date < $2 - 7 GROUP BY c.id, c.name").bind(c).bind(today).fetch_all(&mut *conn).await?;
    for (id, name, amt) in overdue {
        notifications::notify_roles(conn, c, &["accountant", "sales"], &Note { severity: "warning", title: &format!("{name} owes Tk {amt} past due"), title_bn: Some("বকেয়া আদায় প্রয়োজন"), body: "More than a week overdue. Follow up for payment.", entity: Some("customers"), entity_id: Some(id), dedupe: Some(format!("overdue:{id}:{}", today.format("%G-%V"))) }).await?;
    }

    let bills: Vec<(Uuid, String, String, NaiveDate)> = sqlx::query_as("SELECT b.id, b.bill_no, s.name, b.due_date FROM supplier_bills b JOIN suppliers s ON s.id = b.supplier_id WHERE b.company_id = $1 AND b.status IN ('open','partial') AND b.due_date <= $2 + 3").bind(c).bind(today).fetch_all(&mut *conn).await?;
    for (id, no, sup, due) in bills {
        notifications::notify_roles(conn, c, &["accountant"], &Note { severity: if due < today { "critical" } else { "info" }, title: &format!("Bill {no} from {sup} is {}", if due < today { "overdue" } else { "due soon" }), title_bn: None, body: &format!("Due {due}"), entity: Some("supplier_bills"), entity_id: Some(id), dedupe: Some(format!("bill-due:{id}:{}", if due < today { "late" } else { "soon" })) }).await?;
    }
    Ok((count_notes(conn, c).await - before).max(0) as u64)
}

/// Run alerts once at start-up and then every hour.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            match run_alerts(&st).await { Ok(n) => tracing::info!(created = n, "alerts checked"), Err(e) => tracing::warn!(error = %e, "alert job failed") }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}
