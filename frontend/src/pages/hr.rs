//! HR & payroll: employees, departments, daily attendance, leave and monthly payroll.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde_json::{json, Value};

const DEPT_FIELDS: &[FF] = &[ff("code", "Code", FK::Text).req(), ff("name", "Name", FK::Text).req(), ff("name_bn", "Name in Bangla", FK::Text), ff("parent_id", "Under", FK::Lookup("/departments", &["name"])), ff("is_active", "Active", FK::Bool).def("true")];
static DEPTS: ListCfg = ListCfg { title: "Departments", subtitle: "", endpoint: "/departments", cols: &[col("code", "Code", CK::Mono), col("name", "Department", CK::Bold), col("name_bn", "In Bangla", CK::Text), col("headcount", "People", CK::Int), col("is_active", "Active", CK::Bool)], search: false, filters: &[], fields: DEPT_FIELDS, write: "hr.write", new_label: "New department", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };
const EMP_FIELDS: &[FF] = &[ff("name", "Name", FK::Text).req(), ff("name_bn", "Name in Bangla", FK::Text), ff("designation", "Designation", FK::Text), ff("department_id", "Department", FK::Lookup("/departments", &["name"])), ff("branch_id", "Branch", FK::Lookup("/branches", &["name"])), ff("join_date", "Joined", FK::Date).req().def("today"),
    ff("employment_type", "Type", FK::Select(&[("permanent", "Permanent"), ("probation", "Probation"), ("contract", "Contract"), ("intern", "Intern")])).def("permanent"), ff("status", "Status", FK::Select(&[("active", "Active"), ("on_leave", "On leave"), ("resigned", "Resigned"), ("terminated", "Terminated")])).def("active"), ff("phone", "Phone", FK::Text), ff("email", "Email", FK::Email), ff("nid", "National ID", FK::Text), ff("tin", "TIN", FK::Text),
    ff("basic_salary", "Basic salary (Tk)", FK::Dec).def("0"), ff("house_rent", "House rent (Tk)", FK::Dec).def("0"), ff("medical", "Medical (Tk)", FK::Dec).def("0"), ff("conveyance", "Conveyance (Tk)", FK::Dec).def("0"), ff("other_allowance", "Other allowance (Tk)", FK::Dec).def("0"), ff("pf_rate", "Provident fund % of basic", FK::Dec).def("0").help("Deducted from the employee and matched by the company"), ff("monthly_tax", "Monthly income tax (Tk)", FK::Dec).def("0"),
    ff("payment_method", "Paid by", FK::Select(&[("bank", "Bank"), ("cash", "Cash"), ("bkash", "bKash")])).def("bank"), ff("bank_account_no", "Bank account no.", FK::Text), ff("address", "Address", FK::Area).full()];
static EMPLOYEES: ListCfg = ListCfg { title: "Employees", subtitle: "", endpoint: "/employees", cols: &[col("code", "ID", CK::Mono), col("name", "Name", CK::Bold), col("designation", "Designation", CK::Text), col("department_name", "Department", CK::Text), col("join_date", "Joined", CK::Date), col("gross_salary", "Gross salary", CK::Money), col("status", "Status", CK::Badge)],
    search: true, filters: &[Filter { param: "status", label: "All", options: &[("active", "Active"), ("on_leave", "On leave"), ("resigned", "Resigned")] }], fields: EMP_FIELDS, write: "hr.write", new_label: "New employee", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "Add your team.", date_filter: false };
static PAYROLL: ListCfg = ListCfg { title: "Payroll", subtitle: "", endpoint: "/payroll-runs", cols: &[col("run_no", "Run", CK::Bold), col("period_year", "Year", CK::Int), col("period_month", "Month", CK::Int), col("employees", "People", CK::Int), col("total_gross", "Gross", CK::Money), col("total_net", "Net pay", CK::Money), col("status", "Status", CK::Badge)],
    search: false, filters: &[], fields: &[], write: "payroll.write", new_label: "", edit: false, delete: false, detail: "/payroll-runs/", new_route: "", extra_query: "", empty_hint: "Prepare payroll for a month.", date_filter: false };
static LEAVE: ListCfg = ListCfg { title: "Leave", subtitle: "", endpoint: "/leave-requests", cols: &[col("employee_name", "Employee", CK::Bold), col("leave_type", "Type", CK::Badge), col("from_date", "From", CK::Date), col("to_date", "To", CK::Date), col("days", "Days", CK::Qty), col("reason", "Reason", CK::Text), col("status", "Status", CK::Badge)],
    search: false, filters: &[Filter { param: "status", label: "All", options: &[("pending", "Pending"), ("approved", "Approved"), ("rejected", "Rejected")] }], fields: &[ff("employee_id", "Employee", FK::Lookup("/employees?status=active", &["name"])).req(), ff("leave_type", "Type", FK::Select(&[("casual", "Casual"), ("sick", "Sick"), ("earned", "Earned"), ("maternity", "Maternity"), ("unpaid", "Unpaid"), ("other", "Other")])).req().def("casual"), ff("from_date", "From", FK::Date).req().def("today"), ff("to_date", "To", FK::Date).req().def("today"), ff("reason", "Reason", FK::Text).full()],
    write: "hr.write", new_label: "Request leave", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };

#[component]
pub fn Hr() -> impl IntoView {
    let tab = RwSignal::new("emp");
    view! { <div class="page"><PageHead title=t("HR & Payroll") subtitle="Your people, who is in today, leave, and monthly salaries with provident fund and tax."/>
        <Tabs tabs=vec![("emp", t("Employees")), ("att", t("Attendance")), ("leave", t("Leave")), ("pay", t("Payroll")), ("dep", t("Departments"))] active=tab/>
        {move || match tab.get() { "emp" => view! { <ListPage cfg=&EMPLOYEES headless=true/> }.into_any(), "att" => view! { <Attendance/> }.into_any(), "leave" => view! { <LeaveTab/> }.into_any(), "pay" => view! { <PayrollTab/> }.into_any(), _ => view! { <ListPage cfg=&DEPTS headless=true/> }.into_any() }}</div> }
}

#[component]
fn Attendance() -> impl IntoView {
    let date = RwSignal::new(fmt::today()); let tick = state::app().tick;
    let emps = LocalResource::new(move || { let _ = tick.get(); async { api::get("/employees?status=active&per_page=500").await } });
    let existing = LocalResource::new(move || { let d = date.get(); let _ = tick.get(); async move { api::get(&format!("/attendance?from={d}&to={d}&per_page=500")).await } });
    let marks: RwSignal<std::collections::HashMap<String, String>> = RwSignal::new(Default::default());
    Effect::new(move |_| { if let Some(Ok(v)) = existing.get() { marks.set(v["data"].as_array().cloned().unwrap_or_default().iter().map(|a| (fmt::text(&a["employee_id"]), fmt::text(&a["status"]))).collect()); } });
    let save = move |_| { let m = marks.get_untracked(); let rows: Vec<Value> = m.iter().map(|(k, v)| json!({"employee_id": k, "status": v})).collect(); if rows.is_empty() { return; } let d = date.get_untracked();
        leptos::task::spawn_local(async move { match api::post("/attendance", json!({"work_date": d, "rows": rows})).await { Ok(_) => { state::ok("Attendance saved"); state::refresh(); } Err(e) => state::fail(e.friendly()) } }); };
    let all_present = move |_| {
        if let Some(Ok(v)) = emps.get() {
            let ids: Vec<String> = v["data"].as_array().cloned().unwrap_or_default().iter().map(|e| fmt::text(&e["id"])).collect();
            marks.update(|m| { for id in ids { m.entry(id).or_insert_with(|| "present".to_string()); } });
        }
    };
    view! { <div class="card flush"><div class="toolbar"><input class="input" type="date" prop:value=move || date.get() on:change=move |e| date.set(event_target_value(&e))/>
        {move || state::can("hr.write").then(|| view! { <button class="btn sm" on:click=all_present>"Mark everyone present"</button> })}
        {move || state::can("hr.write").then(|| view! { <button class="btn primary sm sp" on:click=save>"Save attendance"</button> })}</div>
        {move || match emps.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default();
            view! { <div class="tbl-wrap"><table><thead><tr><th>"Employee"</th><th>"Department"</th><th>"Status today"</th></tr></thead><tbody>{l.into_iter().map(|e| { let id = fmt::text(&e["id"]); let id2 = id.clone();
                view! { <tr><td><b>{fmt::text(&e["name"])}</b><div class="small dim">{fmt::text(&e["designation"])}</div></td><td>{fmt::text(&e["department_name"])}</td>
                    <td><div class="row wrap">{[("present", "Present"), ("late", "Late"), ("half_day", "Half day"), ("absent", "Absent"), ("leave", "Leave")].into_iter().map(|(k, l)| { let id = id.clone(); let id3 = id2.clone(); view! { <button class=move || if marks.get().get(&id).map(String::as_str) == Some(k) { "chip on" } else { "chip" } on:click=move |_| marks.update(|m| { m.insert(id3.clone(), k.into()); })>{l}</button> } }).collect_view()}</div></td></tr> } }).collect_view()}</tbody></table></div> }.into_any() } }}</div> }
}

#[component]
fn LeaveTab() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/leave-requests?status=pending&per_page=50").await } });
    let decide = move |id: String, approve: bool| leptos::task::spawn_local(async move {
        match api::post(&format!("/leave-requests/{id}/decide"), json!({"approve": approve})).await { Ok(_) => state::ok(if approve { "Leave approved" } else { "Leave rejected" }), Err(e) => state::fail(e.friendly()) }
        state::refresh();
    });
    let pending = move || match res.get() {
        Some(Ok(v)) if v["total"].as_i64().unwrap_or(0) > 0 => {
            let rows = v["data"].as_array().cloned().unwrap_or_default().into_iter().map(|r| {
                let id = fmt::text(&r["id"]); let id2 = id.clone();
                let info = format!("{} · {} to {} · {} days", fmt::nice(r["leave_type"].as_str().unwrap_or("")), fmt::date(&r["from_date"]), fmt::date(&r["to_date"]), fmt::qty(&r["days"]));
                let buttons = state::can("hr.write").then(|| view! {
                    <span class="sp row">
                        <button class="btn sm danger" on:click=move |_| decide(id.clone(), false)>{t("Reject")}</button>
                        <button class="btn sm primary" on:click=move |_| decide(id2.clone(), true)>{t("Approve")}</button>
                    </span> });
                view! { <div class="row" style="padding:8px 0;border-bottom:1px solid var(--line)"><div><b>{fmt::text(&r["employee_name"])}</b><div class="small mut">{info}</div></div>{buttons}</div> }
            }).collect_view();
            view! { <Card title="Waiting for your decision"><div class="col">{rows}</div></Card><div style="height:16px"></div> }.into_any()
        }
        _ => ().into_any(),
    };
    view! { {pending} <ListPage cfg=&LEAVE headless=true/> }
}

#[component]
fn PayrollTab() -> impl IntoView {
    let dialog = RwSignal::new(false);
    view! { <div class="row" style="margin-bottom:14px">{move || state::can("payroll.write").then(|| view! { <button class="btn primary sp" on:click=move |_| dialog.set(true)><Icon name="plus" size=16/>"Prepare payroll"</button> })}</div>
        <ListPage cfg=&PAYROLL headless=true/>{move || dialog.get().then(|| view! { <NewRunDialog on_close=closer(move || dialog.set(false))/> })} }
}
#[component]
fn NewRunDialog(on_close: Callback<()>) -> impl IntoView {
    const F: &[FF] = &[ff("year", "Year", FK::Int).req(), ff("month", "Month (1 to 12)", FK::Int).req(), ff("working_days", "Working days", FK::Int).help("Leave empty to use the company policy")];
    let now = js_sys::Date::new_0(); let st = FormState::new(F, None); st.set("year", now.get_full_year().to_string()); st.set("month", (now.get_month() + 1).to_string()); let nav = use_navigate();
    let go = move |_| { st.busy.set(true); st.err.set(None); let b = payload(F, &st); let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/payroll-runs", b).await; st.done(); match r { Ok(v) => { state::ok("Payroll prepared"); on_close.run(()); nav(&format!("/payroll-runs/{}", v["data"]["id"].as_str().unwrap_or("")), Default::default()); } Err(e) => st.fail(e) } }); };
    view! { <Modal title="Prepare payroll" on_close=on_close footer=footer(move || view! { <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button><button class="btn primary" disabled=move || st.busy.get() on:click=go.clone()>"Prepare"</button> }.into_any())><p class="mut small" style="margin-bottom:14px">"Pay is worked out from each person's salary, absence in the attendance records, provident fund and tax."</p><FormFields fields=F.to_vec() st=st/><FormError st=st/></Modal> }
}

static RUN_DOC: DocCfg = DocCfg { endpoint: "/payroll-runs", list_route: "/hr", list_label: "HR & Payroll", title_key: "run_no", subtitle_key: "", status_key: "status", steps: &[("draft", "Prepared"), ("pending_approval", "Approval"), ("approved", "Approved"), ("posted", "Booked"), ("paid", "Paid")],
    facts: &[fact("Year", "period_year", CK::Int), fact("Month", "period_month", CK::Int), fact("Working days", "working_days", CK::Int), fact("People", "employees", CK::Int), fact("Gross", "total_gross", CK::Money), fact("Deductions", "total_deductions", CK::Money), fact("Net pay", "total_net", CK::Money), fact("Employer PF", "total_employer_pf", CK::Money)],
    tables: &[dtable("Payslips", "payslips", &[col("employee_code", "ID", CK::Mono), col("employee_name", "Employee", CK::Bold), col("absent_days", "Absent", CK::Qty), col("gross", "Gross", CK::Money), col("absent_deduction", "Absence", CK::Money), col("pf_employee", "PF", CK::Money), col("tax", "Tax", CK::Money), col("net_pay", "Net pay", CK::Money), col("payment_method", "Paid by", CK::Badge)])],
    actions: &[
        action("Submit for approval", "/submit", "payroll.write").when(&["draft"]).primary().confirm("Send this payroll for approval?").done("Submitted"),
        action("Book to accounts", "/post", "payroll.write").when(&["approved"]).primary().confirm("Book salary expense, provident fund and tax liabilities?").done("Payroll booked"),
        action("Pay salaries", "/pay", "finance.write").when(&["posted"]).primary().fields(&[ff("bank_account_id", "Pay from", FK::Lookup("/bank-accounts?currency=BDT", &["name"])).req(), ff("payment_date", "Date", FK::Date).def("today")]).done("Salaries paid"),
        action("Delete", "", "payroll.write").when(&["draft", "rejected"]).method("DELETE").danger().confirm("Delete this payroll run?").done("Deleted").goto("/hr"),
    ], notes_key: "" };
#[component] pub fn PayrollPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&RUN_DOC id=id/></div> } }
