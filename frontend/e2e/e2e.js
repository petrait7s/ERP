// End-to-end browser test of the real application (Leptos UI + Axum API + PostgreSQL).
// Needs: the API serving the built web app (STATIC_DIR=frontend/dist), a database loaded with
// backend/scripts/seed_demo.py, and Playwright. Run:  BASE=http://127.0.0.1:8080 node e2e/e2e.js
const { chromium } = require('playwright');
const BASE = process.env.BASE || 'http://127.0.0.1:8080';
const OUT = process.env.OUT || '/tmp/weft-e2e';
require('fs').mkdirSync(OUT, { recursive: true });
let failures = 0;
const check = (ok, what) => { console.log((ok ? 'PASS ' : 'FAIL ') + what); if (!ok) failures++; };

async function session(browser, email, viewport = { width: 1480, height: 900 }) {
  const ctx = await browser.newContext({ viewport });
  const page = await ctx.newPage();
  page.errors = [];
  page.on('pageerror', e => page.errors.push('pageerror: ' + e.message.slice(0, 200)));
  page.on('console', m => { if (m.type() === 'error' && !/status of (401|403|404|422|409)/.test(m.text())) page.errors.push('console: ' + m.text().slice(0, 200)); });
  page.go = async path => { await page.evaluate(x => { history.pushState({}, '', x); dispatchEvent(new PopStateEvent('popstate')); }, path); await page.waitForTimeout(900); };
  await page.goto(BASE + '/');
  await page.waitForSelector('.login');
  await page.fill('input[type=email]', email);
  await page.fill('input[type=password]', 'Demo@12345');
  await page.click('button.primary');
  await page.waitForSelector('.side');
  await page.waitForTimeout(700);
  return page;
}
const navLabels = async p => (await p.$$eval('.nav a span', els => els.map(e => e.textContent.trim())));

(async () => {
  const browser = await chromium.launch();

  // ---------- owner: whole system, every screen loads ----------
  const o = await session(browser, 'owner@petra.example');
  const nav = await navLabels(o);
  check(['Dashboard', 'Imports & Voyages', 'Landed Cost', 'Customs', 'Replenishment', 'Procurement', 'Order Book', 'Inventory & Warehouse', 'Sales & Customers', 'Ledger Studio', 'Accounting & Finance', 'Cash & FX', 'VAT & Tax', 'HR & Payroll', 'Policy & Approvals', 'Reports & Analytics', 'Administration'].every(x => nav.includes(x)), 'owner sees every module in the menu');
  for (const r of ['/imports', '/landed-costs', '/customs', '/replenishment', '/procurement', '/order-book', '/inventory', '/sales', '/ledger', '/accounting', '/cash', '/tax', '/hr', '/policy', '/reports', '/admin']) {
    await o.go(r);
    check(await o.locator('.page h1').count() > 0 && await o.locator('.alert.err').count() === 0, `owner can open ${r}`);
  }

  // ---------- create a purchase order through the form and submit it ----------
  await o.go('/purchase-orders/new');
  const sel = await o.$$('select'); await sel[0].selectOption({ index: 2 });
  const row = (await o.$$('tbody tr'))[0];
  await (await row.$$('select'))[0].selectOption({ index: 1 });
  const inputs = await row.$$('input'); await inputs[0].fill('20'); await inputs[1].fill('1500');
  await o.click('button.primary:has-text("Save draft")'); await o.waitForTimeout(1400);
  check(/\/purchase-orders\/[0-9a-f-]{36}/.test(o.url()), 'PO saved and its page opened');
  check((await o.textContent('.page-head')).includes('Draft'), 'new PO starts as a draft');
  await o.click('button:has-text("Submit for approval")'); await o.waitForTimeout(300);
  await o.click('.modal footer .btn.primary'); await o.waitForTimeout(1200);
  check(/Approved|Waiting/.test(await o.textContent('.page-head')), 'PO submitted (auto-approved below the limit)');
  // blank required field shows a clear message instead of failing silently
  await o.go('/purchase-orders/new');
  await o.click('button.primary:has-text("Save draft")'); await o.waitForTimeout(800);
  check(await o.locator('.err-msg, .alert.err').count() > 0, 'empty PO form shows validation messages');

  // ---------- sales order -> deliver & invoice from the UI ----------
  await o.go('/sales-orders/new');
  const ss = await o.$$('select'); await ss[0].selectOption({ index: 1 });
  const srow = (await o.$$('tbody tr'))[0];
  const lineSelect = o.locator('tbody tr').first().locator('select').first();
  const productValue = await lineSelect.evaluate(sl => [...sl.options].find(opt => opt.textContent.includes('LED-60W')).value);
  await lineSelect.selectOption(productValue);
  const si = await srow.$$('input'); await si[0].fill('5');
  await o.waitForTimeout(300);
  check(parseFloat(await si[1].inputValue()) > 0, 'unit price auto-fills from the product');
  await o.click('button.primary:has-text("Save draft")'); await o.waitForTimeout(1400);
  await o.click('button:has-text("Confirm order")'); await o.waitForTimeout(300);
  await o.click('.modal footer .btn.primary'); await o.waitForTimeout(1300);
  check((await o.textContent('.page-head')).includes('Confirmed'), 'sales order confirmed');
  await o.click('button:has-text("Deliver goods")'); await o.waitForTimeout(600);
  await o.click('.modal footer .btn.primary'); await o.waitForTimeout(1600);
  check(/\/deliveries\//.test(o.url()), 'delivery created and opened');
  check((await o.textContent('.page')).includes('INV-'), 'delivery was invoiced with a Mushak 6.3 invoice');

  // ---------- language and theme ----------
  await o.go('/');
  await o.click('.lang-btn'); await o.waitForTimeout(900);
  check((await navLabels(o)).includes('ড্যাশবোর্ড'), 'Bangla menu after switching language');
  await o.click('.lang-btn'); await o.waitForTimeout(700);
  await o.click('button[title=Theme]'); await o.waitForTimeout(300);
  check(await o.evaluate(() => document.documentElement.dataset.theme) === 'dark', 'dark theme applies');
  await o.click('button[title=Theme]');

  // ---------- global search ----------
  await o.keyboard.press('Control+k'); await o.keyboard.type('apex'); await o.waitForTimeout(900);
  check(await o.locator('.search-pal .it').count() > 0, 'global search finds customers');
  await o.keyboard.press('Escape');
  check(o.errors.length === 0, 'no browser errors for the owner session' + (o.errors.length ? ': ' + o.errors.join(' | ') : ''));

  // ---------- role-based views ----------
  const s = await session(browser, 'sales@petra.example');
  const sn = await navLabels(s);
  check(sn.includes('Sales & Customers') && sn.includes('Order Book'), 'sales sees sales screens');
  check(!sn.includes('Administration') && !sn.includes('HR & Payroll') && !sn.includes('Ledger Studio') && !sn.includes('Imports & Voyages'), 'sales does not see admin, HR, ledger or imports');
  await s.go('/admin');
  check((await s.textContent('.page')).includes('not part of your role'), 'direct link to a forbidden screen is blocked');
  await s.go('/');
  await s.waitForSelector('.kpi', { timeout: 8000 });
  const salesText = await s.textContent('.page');
  if (salesText.includes('Cash & bank')) console.log('   dashboard text:', salesText.slice(0, 300));
  check(await s.locator('.kpi').count() > 0 && !salesText.includes('Cash & bank'), 'sales dashboard shows only sales figures');
  check(s.errors.length === 0, 'no browser errors for the sales session' + (s.errors.length ? ': ' + s.errors.join(' | ') : ''));

  const w = await session(browser, 'store@petra.example');
  const wn = await navLabels(w);
  check(wn.includes('Inventory & Warehouse') && !wn.includes('Accounting & Finance') && !wn.includes('Administration'), 'warehouse staff see stock but not accounts or admin');

  const h = await session(browser, 'hr@petra.example');
  const hn = await navLabels(h);
  check(hn.includes('HR & Payroll') && !hn.includes('Sales & Customers') && !hn.includes('Ledger Studio'), 'HR sees people screens only');
  await h.go('/hr'); await h.click('button:has-text("Payroll")'); await h.waitForTimeout(800);

  // ---------- phone layout ----------
  const m = await session(browser, 'owner@petra.example', { width: 390, height: 800 });
  check(await m.locator('.menu-btn').isVisible(), 'phone layout shows the menu button');
  await m.click('.menu-btn'); await m.waitForTimeout(400);
  check((await m.locator('.side').getAttribute('class')).includes('open'), 'phone menu opens');
  await m.screenshot({ path: OUT + '/mobile-menu.png' });
  const overflow = await m.evaluate(() => document.documentElement.scrollWidth > window.innerWidth + 2);
  check(!overflow, 'no horizontal page scroll on a phone');

  console.log(failures ? `\n${failures} CHECK(S) FAILED` : '\nALL CHECKS PASSED');
  await browser.close();
  process.exit(failures ? 1 : 0);
})();
