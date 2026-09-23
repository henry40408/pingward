//! Page helpers over `WebDriver`, which has no retry layer: the waits live here.
//!
//! * Visible is not present: `no_js.feature` relies on controls that the
//!   stylesheet hides but that remain in the document.
//! * `_opt`/`_all` lookups use `nowait`; wrap them in
//!   [`crate::wait::eventually`] when the page must become that way.

use std::time::Instant;

use anyhow::{Context, Result, bail};
use thirtyfour::components::SelectElement;
use thirtyfour::prelude::*;

use crate::browser::{WAIT_INTERVAL, WAIT_TIMEOUT};

#[allow(async_fn_in_trait)]
pub trait Dom {
    /// Waits for the displayed element with this `data-testid`.
    async fn test_id(&self, id: &str) -> Result<WebElement>;

    async fn test_id_opt(&self, id: &str) -> Result<Option<WebElement>>;

    async fn test_ids(&self, id: &str) -> Result<Vec<WebElement>>;

    /// Waits for the displayed element matching a CSS selector.
    async fn css(&self, selector: &str) -> Result<WebElement>;

    async fn css_opt(&self, selector: &str) -> Result<Option<WebElement>>;

    async fn css_all(&self, selector: &str) -> Result<Vec<WebElement>>;

    async fn fill(&self, id: &str, value: &str) -> Result<()>;

    /// Clicks the element once it is clickable.
    async fn click(&self, id: &str) -> Result<()>;

    async fn expect_visible(&self, id: &str) -> Result<()>;

    /// Waits for the element to be undisplayed; it may stay in the DOM.
    async fn expect_hidden(&self, id: &str) -> Result<()>;

    /// Waits for no element with that id to exist.
    async fn expect_absent(&self, id: &str) -> Result<()>;

    async fn expect_text(&self, id: &str, needle: &str) -> Result<()>;

    /// Exact match, so `up` is not satisfied by a chip reading `paused`.
    async fn expect_exact_text(&self, id: &str, text: &str) -> Result<()>;

    async fn expect_not_exact_text(&self, id: &str, text: &str) -> Result<()>;

    async fn expect_visible_css(&self, selector: &str) -> Result<()>;

    async fn expect_hidden_css(&self, selector: &str) -> Result<()>;

    async fn expect_exact_text_css(&self, selector: &str, text: &str) -> Result<()>;

    async fn expect_text_css(&self, selector: &str, needle: &str) -> Result<()>;

    async fn expect_value(&self, id: &str, value: &str) -> Result<()>;

    async fn expect_value_css(&self, selector: &str, value: &str) -> Result<()>;

    async fn test_ids_with_text(&self, id: &str, text: &str) -> Result<Vec<WebElement>>;

    async fn css_with_text(&self, selector: &str, text: &str) -> Result<Vec<WebElement>>;

    /// Waits for the first selector match whose text contains `text`.
    async fn css_row(&self, selector: &str, text: &str) -> Result<WebElement>;

    /// Chooses an `<option>` by its visible label.
    async fn select_label(&self, id: &str, label: &str) -> Result<()>;

    async fn text_of(&self, id: &str) -> Result<String>;

    /// Whether the element is present and displayed right now.
    async fn is_visible(&self, id: &str) -> Result<bool>;

    async fn expect_count(&self, selector: &str, expected: usize) -> Result<()>;

    async fn count_css(&self, selector: &str) -> Result<usize>;

    async fn fill_css(&self, selector: &str, value: &str) -> Result<()>;

    async fn click_css(&self, selector: &str) -> Result<()>;

    /// Clicks a navigating control and waits for the old document to go stale.
    /// `WebDriver`'s click returns before a redirect lands, and watching the URL
    /// misses forms that redirect back to the same page.
    async fn submit_css(&self, selector: &str) -> Result<()>;

    async fn submit(&self, id: &str) -> Result<()>;

    /// Chooses an `<option>` by value.
    async fn select_option(&self, id: &str, value: &str) -> Result<()>;

    async fn select_option_css(&self, selector: &str, value: &str) -> Result<()>;

    async fn value_of(&self, id: &str) -> Result<String>;

    async fn value_of_css(&self, selector: &str) -> Result<String>;

    /// Waits for `attr` to equal `expected`, or be absent when `None`.
    async fn expect_attr(&self, selector: &str, attr: &str, expected: Option<&str>) -> Result<()>;

    /// Waits for the first `<tr>` containing `text`.
    async fn row_with_text(&self, text: &str) -> Result<WebElement>;

    /// Waits for the innermost element containing `text` to be displayed.
    async fn expect_text_somewhere(&self, text: &str) -> Result<()>;

    async fn has_text_somewhere(&self, text: &str) -> Result<bool>;

    /// Waits for an element whose whole text is `text` to be displayed.
    async fn expect_exact_text_somewhere(&self, text: &str) -> Result<()>;

    async fn link_named(&self, name: &str) -> Result<Option<WebElement>>;

    async fn button_named(&self, name: &str) -> Result<Option<WebElement>>;

    /// The heading with exactly this text, if any.
    async fn heading_opt(&self, name: &str) -> Result<Option<WebElement>>;

    async fn eval(&self, script: &str) -> Result<serde_json::Value>;

    /// Finds and reads in one go; `None` if absent or stale mid-read (a fragment
    /// swap racing the read means "not yet").
    async fn text_of_css(&self, selector: &str) -> Result<Option<String>>;

    async fn text_of_test_id(&self, id: &str) -> Result<Option<String>>;

    async fn texts_of(&self, selector: &str) -> Result<Vec<String>>;

    async fn is_checked(&self, id: &str) -> Result<bool>;

    /// Ticks a checkbox unless already ticked.
    async fn check(&self, id: &str) -> Result<()>;

    async fn computed_style(&self, selector: &str, property: &str) -> Result<String>;

    /// Waits for a `confirm()` prompt, which appears after the click returns,
    /// and reads its message.
    async fn confirm_message(&self) -> Result<String>;

    async fn accept_confirm(&self) -> Result<()>;

    async fn dismiss_confirm(&self) -> Result<()>;

    /// Clicks a `data-confirm` control, accepts, and waits for the submission to
    /// land (these often redirect back to the same page).
    async fn confirm_and_submit(&self, id: &str) -> Result<()>;

    /// Border box `(x, y, width, height)` in viewport coordinates, via
    /// `getBoundingClientRect`: `WebElement::rect` is document-relative.
    async fn bounding_box(&self, selector: &str) -> Result<(f64, f64, f64, f64)>;
}

/// Waits until clickable, then clicks: `WebElement::click` happily clicks a
/// disabled control such as the pager's `<span class="btn disabled">` ends.
pub async fn click_when_ready(element: &WebElement) -> Result<()> {
    element
        .wait_until()
        .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
        .clickable()
        .await
        .context("the element never became clickable")?;
    element.click().await?;
    Ok(())
}

/// [`Dom::submit_css`] for an element already found (e.g. within a row).
pub async fn submit_element(driver: &WebDriver, element: &WebElement) -> Result<()> {
    let document = driver.find(By::Tag("html")).await?;
    click_when_ready(element).await?;
    document
        .wait_until()
        .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
        .stale()
        .await
        .context("the click did not navigate anywhere")?;
    Ok(())
}

/// Clicks a control that *may* raise a `confirm()`, accepts it if so, and
/// waits for the navigation. `/admin`'s row controls confirm only
/// conditionally, so this races the prompt against the page being replaced.
pub async fn submit_element_confirming(driver: &WebDriver, element: &WebElement) -> Result<()> {
    let document = driver.find(By::Tag("html")).await?;
    click_when_ready(element).await?;

    let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
    loop {
        if driver.get_alert_text().await.is_ok() {
            driver.accept_alert().await?;
            break;
        }
        if document.is_present().await.is_ok_and(|present| !present) {
            // Submitted without confirming.
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            bail!("the control neither confirmed nor navigated within {WAIT_TIMEOUT:?}");
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }

    document
        .wait_until()
        .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
        .stale()
        .await
        .context("the confirmed control did not submit")?;
    Ok(())
}

/// `textContent` rather than `WebElement::text`, which applies
/// `text-transform` (an `uppercase` chip reads `DOWN` for markup `down`).
#[allow(async_fn_in_trait)]
pub trait TextContent {
    async fn content_text(&self) -> Result<String>;

    /// `textContent` with whitespace runs collapsed.
    async fn normalized_text(&self) -> Result<String>;
}

impl TextContent for WebElement {
    async fn content_text(&self) -> Result<String> {
        Ok(self.prop("textContent").await?.unwrap_or_default())
    }

    async fn normalized_text(&self) -> Result<String> {
        Ok(normalize(&self.content_text().await?))
    }
}

pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Descendant lookups, for steps scoped to a row, card or section.
#[allow(async_fn_in_trait)]
pub trait Within {
    async fn link_named(&self, name: &str) -> Result<Option<WebElement>>;

    /// Exact text match, so a row's lowercase `edit` link does not collide
    /// with the page's `Edit`.
    async fn link_named_exact(&self, name: &str) -> Result<Option<WebElement>>;

    async fn button_named(&self, name: &str) -> Result<Option<WebElement>>;

    async fn test_id_opt(&self, id: &str) -> Result<Option<WebElement>>;

    /// Waits for the descendant.
    async fn test_id(&self, id: &str) -> Result<WebElement>;

    async fn css_all(&self, selector: &str) -> Result<Vec<WebElement>>;

    async fn css_opt(&self, selector: &str) -> Result<Option<WebElement>>;
}

impl Within for WebElement {
    async fn link_named(&self, name: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::XPath(named_role_xpath("a", name)))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn link_named_exact(&self, name: &str) -> Result<Option<WebElement>> {
        let xpath = format!(".//a[normalize-space(.)={}]", xpath_literal(name));
        Ok(self.query(By::XPath(xpath)).nowait().first_opt().await?)
    }

    async fn button_named(&self, name: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::XPath(named_role_xpath("button", name)))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn test_id_opt(&self, id: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::Testid(id.to_owned()))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn test_id(&self, id: &str) -> Result<WebElement> {
        self.query(By::Testid(id.to_owned()))
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .first()
            .await
            .with_context(|| format!("no descendant with testid `{id}`"))
    }

    async fn css_all(&self, selector: &str) -> Result<Vec<WebElement>> {
        Ok(self
            .query(By::Css(selector.to_owned()))
            .nowait()
            .all_from_selector()
            .await?)
    }

    async fn css_opt(&self, selector: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::Css(selector.to_owned()))
            .nowait()
            .first_opt()
            .await?)
    }
}

impl Dom for WebDriver {
    async fn test_id(&self, id: &str) -> Result<WebElement> {
        displayed(self, By::Testid(id.to_owned()), &format!("testid `{id}`")).await
    }

    async fn test_id_opt(&self, id: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::Testid(id.to_owned()))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn test_ids(&self, id: &str) -> Result<Vec<WebElement>> {
        all(self, By::Testid(id.to_owned())).await
    }

    async fn css(&self, selector: &str) -> Result<WebElement> {
        displayed(
            self,
            By::Css(selector.to_owned()),
            &format!("selector `{selector}`"),
        )
        .await
    }

    async fn css_opt(&self, selector: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::Css(selector.to_owned()))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn css_all(&self, selector: &str) -> Result<Vec<WebElement>> {
        all(self, By::Css(selector.to_owned())).await
    }

    async fn fill(&self, id: &str, value: &str) -> Result<()> {
        let field = self.test_id(id).await?;
        fill_element(self, &field, value).await
    }

    async fn click(&self, id: &str) -> Result<()> {
        self.query(By::Testid(id.to_owned()))
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .and_clickable()
            .first()
            .await
            .with_context(|| format!("no clickable element with testid `{id}`"))?
            .click()
            .await?;
        Ok(())
    }

    async fn expect_visible(&self, id: &str) -> Result<()> {
        self.test_id(id).await.map(|_| ())
    }

    async fn expect_hidden(&self, id: &str) -> Result<()> {
        crate::wait::eventually(&format!("testid `{id}` is hidden"), || async {
            match self.test_id_opt(id).await? {
                None => Ok(true),
                Some(element) => Ok(!element.is_displayed().await.unwrap_or(false)),
            }
        })
        .await
    }

    async fn expect_absent(&self, id: &str) -> Result<()> {
        crate::wait::eventually(&format!("testid `{id}` is gone"), || async {
            Ok(self.test_ids(id).await?.is_empty())
        })
        .await
    }

    async fn expect_text(&self, id: &str, needle: &str) -> Result<()> {
        // Hand-rolled so the failure can name the last text seen.
        let deadline = Instant::now() + WAIT_TIMEOUT;
        let mut last = None;
        loop {
            if let Some(element) = self.test_id_opt(id).await?
                && let Ok(text) = element.normalized_text().await
            {
                if text.contains(needle) {
                    return Ok(());
                }
                last = Some(text);
            }
            if Instant::now() >= deadline {
                let seen =
                    last.map_or_else(|| "no such element".to_owned(), |text| format!("{text:?}"));
                bail!(
                    "testid `{id}`: expected text containing {needle:?}, \
                     last saw {seen} after {WAIT_TIMEOUT:?}"
                );
            }
            tokio::time::sleep(WAIT_INTERVAL).await;
        }
    }

    async fn expect_exact_text(&self, id: &str, text: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("testid `{id}` text"), text.to_owned(), || {
            self.text_of(id)
        })
        .await
    }

    async fn expect_not_exact_text(&self, id: &str, text: &str) -> Result<()> {
        crate::wait::eventually(&format!("testid `{id}` stops reading {text:?}"), || async {
            Ok(self.text_of_test_id(id).await?.as_deref() != Some(text))
        })
        .await
    }

    async fn expect_visible_css(&self, selector: &str) -> Result<()> {
        self.css(selector).await.map(|_| ())
    }

    async fn expect_hidden_css(&self, selector: &str) -> Result<()> {
        crate::wait::eventually(&format!("`{selector}` is hidden"), || async {
            match self.css_opt(selector).await? {
                None => Ok(true),
                Some(element) => Ok(!element.is_displayed().await.unwrap_or(false)),
            }
        })
        .await
    }

    async fn expect_exact_text_css(&self, selector: &str, text: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("`{selector}` text"), text.to_owned(), || async {
            Ok(self.text_of_css(selector).await?.unwrap_or_default())
        })
        .await
    }

    async fn expect_text_css(&self, selector: &str, needle: &str) -> Result<()> {
        crate::wait::eventually(
            &format!("`{selector}` text contains {needle:?}"),
            || async {
                Ok(self
                    .text_of_css(selector)
                    .await?
                    .is_some_and(|text| text.contains(needle)))
            },
        )
        .await
    }

    async fn expect_value(&self, id: &str, value: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("testid `{id}` value"), value.to_owned(), || {
            self.value_of(id)
        })
        .await
    }

    async fn expect_value_css(&self, selector: &str, value: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("`{selector}` value"), value.to_owned(), || {
            self.value_of_css(selector)
        })
        .await
    }

    async fn test_ids_with_text(&self, id: &str, text: &str) -> Result<Vec<WebElement>> {
        with_text(self.test_ids(id).await?, text).await
    }

    async fn css_with_text(&self, selector: &str, text: &str) -> Result<Vec<WebElement>> {
        with_text(self.css_all(selector).await?, text).await
    }

    async fn css_row(&self, selector: &str, text: &str) -> Result<WebElement> {
        crate::wait::eventually_some(&format!("`{selector}` containing {text:?}"), || async {
            Ok(self.css_with_text(selector, text).await?.into_iter().next())
        })
        .await
    }

    async fn select_label(&self, id: &str, label: &str) -> Result<()> {
        let select = SelectElement::new(&self.test_id(id).await?).await?;
        select
            .select_by_exact_text(label)
            .await
            .with_context(|| format!("`{id}` has no option labelled `{label}`"))?;
        Ok(())
    }

    async fn text_of(&self, id: &str) -> Result<String> {
        self.test_id(id).await?.normalized_text().await
    }

    async fn is_visible(&self, id: &str) -> Result<bool> {
        match self.test_id_opt(id).await? {
            None => Ok(false),
            Some(element) => Ok(element.is_displayed().await.unwrap_or(false)),
        }
    }

    async fn expect_count(&self, selector: &str, expected: usize) -> Result<()> {
        crate::wait::eventually_eq(&format!("`{selector}` count"), expected, || {
            self.count_css(selector)
        })
        .await
    }

    async fn count_css(&self, selector: &str) -> Result<usize> {
        Ok(self.css_all(selector).await?.len())
    }

    async fn fill_css(&self, selector: &str, value: &str) -> Result<()> {
        let field = self.css(selector).await?;
        fill_element(self, &field, value).await
    }

    async fn click_css(&self, selector: &str) -> Result<()> {
        self.query(By::Css(selector.to_owned()))
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .and_clickable()
            .first()
            .await
            .with_context(|| format!("nothing clickable matches `{selector}`"))?
            .click()
            .await?;
        Ok(())
    }

    async fn submit_css(&self, selector: &str) -> Result<()> {
        let document = self.find(By::Tag("html")).await?;
        self.click_css(selector).await?;
        document
            .wait_until()
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .stale()
            .await
            .with_context(|| format!("`{selector}` did not navigate anywhere"))?;
        Ok(())
    }

    async fn submit(&self, id: &str) -> Result<()> {
        let document = self.find(By::Tag("html")).await?;
        self.click(id).await?;
        document
            .wait_until()
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .stale()
            .await
            .with_context(|| format!("`{id}` did not navigate anywhere"))?;
        Ok(())
    }

    async fn select_option(&self, id: &str, value: &str) -> Result<()> {
        select_value(&self.test_id(id).await?, id, value).await
    }

    async fn select_option_css(&self, selector: &str, value: &str) -> Result<()> {
        select_value(&self.css(selector).await?, selector, value).await
    }

    async fn value_of(&self, id: &str) -> Result<String> {
        value_of_element(&self.test_id(id).await?).await
    }

    async fn value_of_css(&self, selector: &str) -> Result<String> {
        value_of_element(&self.css(selector).await?).await
    }

    async fn expect_attr(&self, selector: &str, attr: &str, expected: Option<&str>) -> Result<()> {
        let what = match expected {
            Some(value) => format!("`{selector}` has {attr}={value:?}"),
            None => format!("`{selector}` has no {attr}"),
        };
        crate::wait::eventually(&what, || async {
            let Some(element) = self.css_opt(selector).await? else {
                return Ok(false);
            };
            // Stale mid-read means the document was replaced: "not yet".
            match element.attr(attr).await {
                Ok(value) => Ok(value.as_deref() == expected),
                Err(_) => Ok(false),
            }
        })
        .await
    }

    async fn row_with_text(&self, text: &str) -> Result<WebElement> {
        let xpath = format!("//tr[contains(., {})]", xpath_literal(text));
        self.query(By::XPath(xpath))
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .first()
            .await
            .with_context(|| format!("no table row contains {text:?}"))
    }

    async fn expect_text_somewhere(&self, text: &str) -> Result<()> {
        displayed(
            self,
            By::XPath(innermost_text_xpath(text)),
            &format!("text {text:?}"),
        )
        .await
        .map(|_| ())
    }

    async fn has_text_somewhere(&self, text: &str) -> Result<bool> {
        let Some(element) = self
            .query(By::XPath(innermost_text_xpath(text)))
            .nowait()
            .first_opt()
            .await?
        else {
            return Ok(false);
        };
        Ok(element.is_displayed().await.unwrap_or(false))
    }

    async fn expect_exact_text_somewhere(&self, text: &str) -> Result<()> {
        let xpath = format!(
            "//*[normalize-space(.)={0}][not(.//*[normalize-space(.)={0}])]",
            xpath_literal(text)
        );
        displayed(self, By::XPath(xpath), &format!("exactly {text:?}"))
            .await
            .map(|_| ())
    }

    async fn link_named(&self, name: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::XPath(named_role_xpath("a", name)))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn button_named(&self, name: &str) -> Result<Option<WebElement>> {
        Ok(self
            .query(By::XPath(named_role_xpath("button", name)))
            .nowait()
            .first_opt()
            .await?)
    }

    async fn heading_opt(&self, name: &str) -> Result<Option<WebElement>> {
        // `normalize-space` matches `exact: true`, which compares
        // whitespace-normalised text rather than the raw node value.
        let xpath = format!(
            "//*[self::h1 or self::h2 or self::h3 or self::h4 or self::h5 or self::h6]\
             [normalize-space(.)={}]",
            xpath_literal(name)
        );
        Ok(self.query(By::XPath(xpath)).nowait().first_opt().await?)
    }

    async fn eval(&self, script: &str) -> Result<serde_json::Value> {
        Ok(self.execute(script, vec![]).await?.json().clone())
    }

    async fn text_of_css(&self, selector: &str) -> Result<Option<String>> {
        let Some(element) = self.css_opt(selector).await? else {
            return Ok(None);
        };
        Ok(element.normalized_text().await.ok())
    }

    async fn text_of_test_id(&self, id: &str) -> Result<Option<String>> {
        let Some(element) = self.test_id_opt(id).await? else {
            return Ok(None);
        };
        Ok(element.normalized_text().await.ok())
    }

    async fn texts_of(&self, selector: &str) -> Result<Vec<String>> {
        let mut texts = Vec::new();
        for element in self.css_all(selector).await? {
            texts.push(element.normalized_text().await?);
        }
        Ok(texts)
    }

    async fn is_checked(&self, id: &str) -> Result<bool> {
        Ok(self.test_id(id).await?.is_selected().await?)
    }

    async fn check(&self, id: &str) -> Result<()> {
        if !self.is_checked(id).await? {
            self.click(id).await?;
        }
        Ok(())
    }

    async fn computed_style(&self, selector: &str, property: &str) -> Result<String> {
        // Not `css`, which waits for display: callers often probe `display: none`.
        let element = self
            .css_opt(selector)
            .await?
            .with_context(|| format!("no element matches `{selector}`"))?;
        let value = self
            .execute(
                "return getComputedStyle(arguments[0]).getPropertyValue(arguments[1]);",
                vec![element.to_json()?, serde_json::json!(property)],
            )
            .await?;
        Ok(value.json().as_str().unwrap_or_default().to_owned())
    }

    async fn confirm_message(&self) -> Result<String> {
        crate::wait::eventually_some("a confirm() prompt", || async {
            Ok(self.get_alert_text().await.ok())
        })
        .await
    }

    async fn accept_confirm(&self) -> Result<()> {
        // Waits out the gap between the click returning and the prompt.
        self.confirm_message().await?;
        self.accept_alert().await?;
        Ok(())
    }

    async fn dismiss_confirm(&self) -> Result<()> {
        self.confirm_message().await?;
        self.dismiss_alert().await?;
        Ok(())
    }

    async fn confirm_and_submit(&self, id: &str) -> Result<()> {
        let document = self.find(By::Tag("html")).await?;
        self.click(id).await?;
        self.accept_confirm().await?;
        document
            .wait_until()
            .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
            .stale()
            .await
            .with_context(|| format!("`{id}` did not submit after the prompt was accepted"))?;
        Ok(())
    }

    async fn bounding_box(&self, selector: &str) -> Result<(f64, f64, f64, f64)> {
        let rect = self
            .execute(
                "const r = arguments[0].getBoundingClientRect();\
                 return [r.x, r.y, r.width, r.height];",
                vec![self.css(selector).await?.to_json()?],
            )
            .await?;
        let values = rect
            .json()
            .as_array()
            .context("the rect probe did not return an array")?
            .iter()
            .map(|value| value.as_f64().unwrap_or_default())
            .collect::<Vec<_>>();
        let [x, y, width, height] = values[..] else {
            bail!(
                "the rect probe returned {} values, expected 4",
                values.len()
            );
        };
        Ok((x, y, width, height))
    }
}

/// Replaces a field's contents with `clear` + `send_keys` (which appends).
/// Date/time inputs are set via `value` instead: `clear` leaves a
/// `datetime-local` partly filled and refusing keystrokes.
async fn fill_element(driver: &WebDriver, field: &WebElement, value: &str) -> Result<()> {
    let kind = field.attr("type").await?.unwrap_or_default();
    if kind == "datetime-local" || kind == "date" || kind == "time" {
        set_value_via_script(driver, field, value).await?;
        return Ok(());
    }
    field.clear().await?;
    if !value.is_empty() {
        field.send_keys(value).await?;
    }
    Ok(())
}

/// Sets `value` and fires `input`/`change`, which the filter forms listen for.
async fn set_value_via_script(driver: &WebDriver, field: &WebElement, value: &str) -> Result<()> {
    driver
        .execute(
            "const el = arguments[0];\
             el.value = arguments[1];\
             el.dispatchEvent(new Event('input', { bubbles: true }));\
             el.dispatchEvent(new Event('change', { bubbles: true }));",
            vec![field.to_json()?, serde_json::json!(value)],
        )
        .await?;
    Ok(())
}

async fn select_value(element: &WebElement, what: &str, value: &str) -> Result<()> {
    let select = SelectElement::new(element).await?;
    select
        .select_by_value(value)
        .await
        .with_context(|| format!("`{what}` has no option with value `{value}`"))?;
    Ok(())
}

/// `prop`, not `attr`: the attribute is only the initial value.
async fn value_of_element(element: &WebElement) -> Result<String> {
    Ok(element.prop("value").await?.unwrap_or_default())
}

async fn displayed(driver: &WebDriver, by: By, what: &str) -> Result<WebElement> {
    driver
        .query(by)
        .wait(WAIT_TIMEOUT, WAIT_INTERVAL)
        .and_displayed()
        .first()
        .await
        .with_context(|| format!("no displayed element for {what}"))
}

/// Every current match, without waiting; none is an empty `Vec`.
async fn all(driver: &WebDriver, by: By) -> Result<Vec<WebElement>> {
    Ok(driver.query(by).nowait().all_from_selector().await?)
}

/// Keeps elements whose text contains `text`; stale ones (fragment swap in
/// flight) are dropped rather than failing.
async fn with_text(elements: Vec<WebElement>, text: &str) -> Result<Vec<WebElement>> {
    let mut matched = Vec::new();
    for element in elements {
        if let Ok(content) = element.normalized_text().await
            && content.contains(text)
        {
            matched.push(element);
        }
    }
    Ok(matched)
}

/// `XPath` for the innermost element containing `text`; without `not(.//*[…])`
/// every ancestor up to `<body>` would match too.
fn innermost_text_xpath(text: &str) -> String {
    let literal = xpath_literal(text);
    format!(
        "//*[contains(normalize-space(.), {literal})]\
         [not(.//*[contains(normalize-space(.), {literal})])]"
    )
}

/// `XPath` for a `tag` whose text or `aria-label` contains `name`,
/// case-insensitively (`XPath` 1.0 needs `translate` to fold case), so
/// "Send test" finds a button labelled "Send test notification".
fn named_role_xpath(tag: &str, name: &str) -> String {
    const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
    let needle = xpath_literal(&name.to_lowercase());
    let fold = |expression: &str| format!("translate({expression}, '{UPPER}', '{LOWER}')");
    format!(
        ".//{tag}[contains({}, {needle}) or contains({}, {needle})]",
        fold("normalize-space(.)"),
        fold("@aria-label"),
    )
}

/// Quotes a string for `XPath`, which has no escapes; a value with both quote
/// kinds is built with `concat()`.
fn xpath_literal(value: &str) -> String {
    if !value.contains('\'') {
        return format!("'{value}'");
    }
    if !value.contains('"') {
        return format!("\"{value}\"");
    }
    let parts: Vec<String> = value.split('\'').map(|part| format!("'{part}'")).collect();
    format!("concat({})", parts.join(", \"'\", "))
}
