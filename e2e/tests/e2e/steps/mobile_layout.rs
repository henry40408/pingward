//! Phone-width layout invariants, asserted by measurement rather than markup
//! because what is tested lives entirely in CSS.
//!
//! The probes are raw strings with real newlines so their own comments survive;
//! a `\` continuation would fold them onto one line and comment out the rest.

use std::time::Duration;

use anyhow::{Result, ensure};
use cucumber::{then, when};
use pingward_e2e::browser::Viewport;
use pingward_e2e::dom::{Dom, submit_element};
use pingward_e2e::wait::eventually_within;
use pingward_e2e::world::PingwardWorld;

/// The three admin health tables, which only render once there is failing
/// data.
const HEALTH_TABLE_IDS: [&str; 3] = ["health-down", "health-channels", "health-recent"];

/// How long to keep reloading while the failure notification lands.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);

#[when(expr = "I view the site at {int}px wide")]
async fn view_at_width(world: &mut PingwardWorld, width: u32) -> Result<()> {
    world.resize(Viewport::new(width, 667)).await
}

/// Reads a number out of a probe's result object.
fn number(value: &serde_json::Value, field: &str) -> f64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default()
}

/// Reads a whole number out of a probe's result object. Line-box and rect
/// counts are integers; comparing them as floats would be `f64` equality.
fn count(value: &serde_json::Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}

/// Reads a string out of a probe's result object.
fn text(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[then("the page has no horizontal scrollbar")]
async fn no_horizontal_scrollbar(world: &mut PingwardWorld) -> Result<()> {
    let measured = world
        .driver()?
        .eval(
            r"return {
                scrollW: document.documentElement.scrollWidth,
                clientW: document.documentElement.clientWidth,
              };",
        )
        .await?;
    let (scroll, client) = (number(&measured, "scrollW"), number(&measured, "clientW"));
    ensure!(
        scroll <= client,
        "page scrolls horizontally: scrollWidth {scroll}px > viewport {client}px"
    );
    Ok(())
}

#[then("only the users table scrolls sideways, not the card around it")]
async fn users_table_contained(world: &mut PingwardWorld) -> Result<()> {
    // A card body sets its own `overflow-x`, so a table placed directly in one
    // scrolls the whole body and drags the Add-user form off-screen with it.
    // `.tscroll` confines the overflow to the table.
    let measured = world
        .driver()?
        .eval(
            r#"const cb = document.querySelector('[data-testid="user-submit"]').closest(".cb");
               const table = cb.querySelector("table");
               return {
                 bodyOverflow: cb.scrollWidth - cb.clientWidth,
                 wrapper: table.parentElement.className,
               };"#,
        )
        .await?;
    // Symptom first so a failure names it; the wrapper check is a diagnostic.
    let overflow = number(&measured, "bodyOverflow");
    ensure!(
        overflow <= 0.0,
        "the card body itself scrolls by {overflow}px, so the Add-user form moves with the table"
    );
    let wrapper = text(&measured, "wrapper");
    ensure!(
        wrapper.contains("tscroll"),
        "the users table is not wrapped in .tscroll (wrapper class: {wrapper:?})"
    );
    Ok(())
}

#[then("Environment rows do not wrap")]
async fn environment_rows_do_not_wrap(world: &mut PingwardWorld) -> Result<()> {
    // The Environment table already scrolls inside `.tscroll`, so wrapping its
    // cells only costs height: a breakable database URL made one row 331px tall.
    //
    // Two assertions with different failure modes. The one-line-box check is
    // exact and font-independent; the height bound is not (58px on macOS, 78px
    // in Linux CI), so it is set from the defect side rather than the fixed one.
    let measured = world
        .driver()?
        .eval(
            r#"const rows = [...document.querySelectorAll('tr[data-testid^="env-row-"]')];
               const tallest = rows.reduce((worst, r) => {
                 const h = Math.round(r.getBoundingClientRect().height);
                 return h > worst.h ? { h, id: r.dataset.testid } : worst;
               }, { h: 0, id: "none" });
               const code = document
                 .querySelector('tr[data-testid="env-row-DATABASE_URL"]')
                 .querySelector("code");
               return {
                 tallestHeight: tallest.h,
                 tallestId: tallest.id,
                 valueLines: code.getClientRects().length,
               };"#,
        )
        .await?;
    let lines = count(&measured, "valueLines");
    ensure!(
        lines == 1,
        "the DATABASE_URL value spans {lines} lines — it is wrapping"
    );
    let (height, id) = (
        number(&measured, "tallestHeight"),
        text(&measured, "tallestId"),
    );
    ensure!(
        height <= 120.0,
        "{id} is {height}px tall — the description column is being squeezed"
    );
    Ok(())
}

#[then("the admin health tables are shown")]
async fn health_tables_shown(world: &mut PingwardWorld) -> Result<()> {
    // The tables only render once there is failing data, and delivery is a
    // background `tokio::spawn`, so the notification can land after this page
    // load. Polling keeps the next step's failure from meaning "not there yet".
    let driver = world.driver()?;
    eventually_within(HEALTH_TIMEOUT, "the admin health tables", || async {
        driver.refresh().await?;
        for id in HEALTH_TABLE_IDS {
            if !driver.is_visible(id).await? {
                return Ok(false);
            }
        }
        Ok(true)
    })
    .await
}

#[then("each admin health table scrolls inside its card, not the card around it")]
async fn health_tables_contained(world: &mut PingwardWorld) -> Result<()> {
    // Containment, for all three tables: the wrapper must be `.tscroll`, it
    // must actually overflow (or containment is vacuous), and the card body
    // must not scroll.
    //
    // Wrapper checks come before the body check here, unlike the users table:
    // the two Notification health tables share one `.cb`, so the shared-body
    // symptom cannot say which table caused it.
    let results = world
        .driver()?
        .execute(
            r#"return arguments[0].map((id) => {
                 const table = document.querySelector(`[data-testid="${id}"]`);
                 const wrapper = table.parentElement;
                 const cb = table.closest(".cb");
                 return {
                   id,
                   wrapperClass: wrapper.className,
                   bodyOverflow: cb.scrollWidth - cb.clientWidth,
                   wrapperOverflow: wrapper.scrollWidth - wrapper.clientWidth,
                 };
               });"#,
            vec![serde_json::json!(HEALTH_TABLE_IDS)],
        )
        .await?
        .json()
        .as_array()
        .cloned()
        .unwrap_or_default();
    ensure!(
        results.len() == HEALTH_TABLE_IDS.len(),
        "the containment probe measured {} of {} tables",
        results.len(),
        HEALTH_TABLE_IDS.len()
    );
    for result in &results {
        let id = text(result, "id");
        let class = text(result, "wrapperClass");
        ensure!(
            class.contains("tscroll"),
            "{id}: table is not wrapped in .tscroll (wrapper class: {class:?})"
        );
        let overflow = number(result, "wrapperOverflow");
        ensure!(
            overflow > 0.0,
            "{id}: the table's own wrapper does not overflow ({overflow}px) — \
             the seeded content is not wide enough to prove containment"
        );
    }
    for result in &results {
        let id = text(result, "id");
        let overflow = number(result, "bodyOverflow");
        ensure!(
            overflow <= 0.0,
            "{id}: the card body itself scrolls by {overflow}px, dragging sibling content with it"
        );
    }
    Ok(())
}

#[then("the heartbeat legend sits on its own row below the edge captions")]
async fn heartbeat_legend_below_captions(world: &mut PingwardWorld) -> Result<()> {
    // Range-based line-box counting: these captions are flex items, so
    // measuring the element itself reports one rect however its text wraps.
    //
    // One line each is the reported symptom; the legend starting *below* them
    // is what distinguishes fixed from broken, since without the full-width row
    // its top sits level with theirs.
    let measured = world
        .driver()?
        .eval(
            r#"const cap = document.querySelector(".beatcap");
               const lineBoxes = (el) => {
                 const range = document.createRange();
                 range.selectNodeContents(el);
                 return range.getClientRects().length;
               };
               const key = cap.querySelector(".key");
               const edges = [...cap.children].filter((el) => el !== key);
               return {
                 edges: edges.map((el) => ({ text: el.textContent, lines: lineBoxes(el) })),
                 edgeBottom: Math.round(Math.max(...edges.map((el) =>
                   el.getBoundingClientRect().bottom))),
                 keyTop: Math.round(key.getBoundingClientRect().top),
               };"#,
        )
        .await?;
    for edge in measured
        .get("edges")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let (caption, lines) = (text(&edge, "text"), count(&edge, "lines"));
        ensure!(lines == 1, "{caption:?} spans {lines} lines — it wrapped");
    }
    let (key_top, edge_bottom) = (number(&measured, "keyTop"), number(&measured, "edgeBottom"));
    ensure!(
        key_top >= edge_bottom,
        "the legend starts at y={key_top}, above the edge captions' bottom edge \
         ({edge_bottom}) — it is still sharing their row"
    );
    Ok(())
}

#[when("I open the project from the breadcrumb")]
async fn open_project_from_breadcrumb(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    let crumbs = driver.css_all(".crumb a").await?;
    let link = crumbs
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("the breadcrumb has fewer than two links"))?;
    submit_element(driver, link).await?;
    world.expect_path_matching(r"/projects/\d+$").await
}

#[then("the check row's status dot sits next to the name")]
async fn status_dot_next_to_name(world: &mut PingwardWorld) -> Result<()> {
    // `.check` hard-coded `dashboard.html`'s child list, so `project.html`'s
    // extra child wrapped the badge onto a second grid row, widening the
    // auto-sized first column and stranding the dot ~74px from the name. The
    // row's own gap is 16px.
    let gap = world
        .driver()?
        .eval(
            r#"const row = document.querySelector(".check");
               const dot = row.querySelector(".status-dot").getBoundingClientRect();
               const meta = row.querySelector(".cmeta").getBoundingClientRect();
               return Math.round(meta.left - dot.right);"#,
        )
        .await?
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("the gap probe did not return a number"))?;
    ensure!(gap <= 20.0, ".status-dot sits {gap}px from .cmeta");
    Ok(())
}

#[then("the group header's count and manage link each stay on one line")]
async fn group_header_labels_unwrapped(world: &mut PingwardWorld) -> Result<()> {
    // Line-box counting via a Range over the contents: a flex item is
    // blockified, so `getClientRects()` on the element reports one rect however
    // its text wraps, while a Range reports one rect per line box.
    //
    // The description must also be truncating, or a header that simply fits
    // satisfies the count and link checks vacuously.
    let measured = world
        .driver()?
        .eval(
            r#"const gh = document.querySelector(".group > .gh");
               const lineBoxes = (sel) => {
                 const range = document.createRange();
                 range.selectNodeContents(gh.querySelector(sel));
                 return range.getClientRects().length;
               };
               const truncated = (sel) => {
                 const e = gh.querySelector(sel);
                 return e.scrollWidth > e.clientWidth;
               };
               return {
                 count: lineBoxes(".count"),
                 link: lineBoxes("a"),
                 descTruncated: truncated(".gdesc"),
                 nameTruncated: truncated("h2"),
               };"#,
        )
        .await?;
    let flag = |name: &str| {
        measured
            .get(name)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default()
    };
    ensure!(
        flag("descTruncated"),
        "the description is not being truncated, so the header is not under width pressure \
         and the checks below prove nothing"
    );
    let lines = count(&measured, "count");
    ensure!(lines == 1, "\"N checks\" spans {lines} lines — it wrapped");
    let link = count(&measured, "link");
    ensure!(link == 1, "\"Manage →\" spans {link} lines — it wrapped");
    // Pinning the labels must not be paid for out of the project name.
    ensure!(
        !flag("nameTruncated"),
        "the project name is truncated — the description should have absorbed the whole squeeze"
    );
    Ok(())
}

#[then(expr = "the check row's name stays on one line beside the {string} chip")]
async fn name_stays_beside_chip(world: &mut PingwardWorld, chip: String) -> Result<()> {
    // Range-based measurement again, but counted by *distinct line tops*: `.nm`
    // wraps its text in the row's real link, and a Range spanning an element
    // yields a rect for the element box on top of the one for its text — two
    // rects at the same y, still one line. The raw count rides along in the
    // failure message so a genuine wrap stays distinguishable.
    //
    // The chip assertion is the non-vacuity guard: a row rendering no chip
    // trivially leaves the name on one line.
    let measured = world
        .driver()?
        .eval(
            r#"const row = document.querySelector(".check");
               const nm = row.querySelector(".cmeta .nm");
               const range = document.createRange();
               range.selectNodeContents(nm);
               const badge = row.querySelector('[data-testid="check-no-channel"]');
               const box = badge && badge.getBoundingClientRect();
               const rects = [...range.getClientRects()];
               return {
                 lines: new Set(rects.map((r) => Math.round(r.top))).size,
                 rects: rects.length,
                 name: nm.textContent,
                 chip: badge && badge.textContent,
                 chipWidth: box ? Math.round(box.width) : 0,
                 chipTop: box ? Math.round(box.top) : 0,
                 metaBottom: Math.round(
                   row.querySelector(".cmeta").getBoundingClientRect().bottom),
               };"#,
        )
        .await?;
    let rendered_chip = text(&measured, "chip");
    ensure!(
        rendered_chip == chip,
        "the row renders no chip, so the line count below is vacuous"
    );
    let chip_width = number(&measured, "chipWidth");
    ensure!(
        chip_width > 0.0,
        "the {chip:?} chip is in the DOM but has no box — it is still display:none at this width"
    );
    let (lines, rects, name) = (
        count(&measured, "lines"),
        count(&measured, "rects"),
        text(&measured, "name"),
    );
    ensure!(
        lines == 1,
        "the check name {name:?} spans {lines} lines ({rects} client rects) — \
         the chip squeezed .cmeta until it wrapped"
    );
    // The name only fits on one line because the chip wrapped onto a row of
    // its own; sharing the row makes the check above hold for short names only.
    let (chip_top, meta_bottom) = (
        number(&measured, "chipTop"),
        number(&measured, "metaBottom"),
    );
    ensure!(
        chip_top >= meta_bottom,
        "the chip starts at y={chip_top}, above the name block's bottom edge \
         ({meta_bottom}) — it is still sharing the row"
    );
    Ok(())
}

#[then("the check row is a single line")]
async fn check_row_single_line(world: &mut PingwardWorld) -> Result<()> {
    // A badge wrapped to another line has its centre far below the dot's; on
    // one line the two coincide.
    let drop = world
        .driver()?
        .eval(
            r#"const row = document.querySelector(".check");
               const centre = (sel) => {
                 const r = row.querySelector(sel).getBoundingClientRect();
                 return r.top + r.height / 2;
               };
               return Math.round(Math.abs(centre(".badge") - centre(".status-dot")));"#,
        )
        .await?
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("the centre probe did not return a number"))?;
    ensure!(
        drop <= 2.0,
        ".badge sits {drop}px below the .status-dot — it wrapped"
    );
    Ok(())
}
