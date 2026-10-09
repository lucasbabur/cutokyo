# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: desktop.spec.ts >> System follows the OS; explicit appearance survives route navigation
- Location: tests/e2e/desktop.spec.ts:314:1

# Error details

```
Error: expect(received).toEqual(expected) // deep equality

- Expected  -  1
+ Received  + 58

- Array []
+ Array [
+   Object {
+     "description": "Ensure the contrast between foreground and background colors meets WCAG 2 AA minimum contrast ratio thresholds",
+     "help": "Elements must meet minimum color contrast ratio thresholds",
+     "helpUrl": "https://dequeuniversity.com/rules/axe/4.13/color-contrast?application=playwright",
+     "id": "color-contrast",
+     "impact": "serious",
+     "nodes": Array [
+       Object {
+         "all": Array [],
+         "any": Array [
+           Object {
+             "data": Object {
+               "bgColor": "#fdfaf4",
+               "contrastRatio": 2.56,
+               "expectedContrastRatio": "4.5:1",
+               "fgColor": "#9e9f98",
+               "fontSize": "8.3pt (11px)",
+               "fontWeight": "bold",
+               "messageKey": null,
+             },
+             "id": "color-contrast",
+             "impact": "serious",
+             "message": "Element has insufficient color contrast of 2.56 (foreground color: #9e9f98, background color: #fdfaf4, font size: 8.3pt (11px), font weight: bold). Expected contrast ratio of 4.5:1",
+             "relatedNodes": Array [
+               Object {
+                 "html": "<section class=\"settings-section\" aria-labelledby=\"privacy-settings-heading\">",
+                 "target": Array [
+                   "section[aria-labelledby=\"privacy-settings-heading\"]",
+                 ],
+               },
+             ],
+           },
+         ],
+         "failureSummary": "Fix any of the following:
+   Element has insufficient color contrast of 2.56 (foreground color: #9e9f98, background color: #fdfaf4, font size: 8.3pt (11px), font weight: bold). Expected contrast ratio of 4.5:1",
+         "html": "<button type=\"button\" class=\"button button--quiet button--small\">Review data handling</button>",
+         "impact": "serious",
+         "none": Array [],
+         "target": Array [
+           ".button--quiet",
+         ],
+       },
+     ],
+     "tags": Array [
+       "cat.color",
+       "wcag2aa",
+       "wcag143",
+       "TTv5",
+       "TT13.c",
+       "EN-301-549",
+       "EN-9.1.4.3",
+       "ACT",
+       "RGAAv4",
+       "RGAA-3.2.1",
+     ],
+   },
+ ]
```

# Page snapshot

```yaml
- generic [ref=e2]:
  - generic [ref=e3]:
    - link "Skip to content" [ref=e4] [cursor=pointer]:
      - /url: "#main-content"
    - complementary "Application" [ref=e5]:
      - strong [ref=e11]: Cutokyo
      - navigation "Primary navigation" [ref=e12]:
        - link "Overview" [ref=e13] [cursor=pointer]:
          - /url: "#/dashboard"
        - link "Sessions" [ref=e18] [cursor=pointer]:
          - /url: "#/sessions"
        - link "Analysis" [ref=e24] [cursor=pointer]:
          - /url: "#/analysis"
        - link "Agent tools" [ref=e29] [cursor=pointer]:
          - /url: "#/inventory"
        - link "Health" [ref=e41] [cursor=pointer]:
          - /url: "#/health"
        - link "Data" [ref=e47] [cursor=pointer]:
          - /url: "#/data"
        - link "Settings" [ref=e53] [cursor=pointer]:
          - /url: "#/settings"
      - generic [ref=e58]:
        - generic [ref=e59]: Local writer
        - generic [ref=e62]:
          - generic [aria-hidden] [ref=e63]: ✓
          - text: Local only
        - generic [ref=e64]: v0.1.0-test
    - generic [ref=e65]:
      - banner [ref=e66]:
        - generic [ref=e67]:
          - generic [ref=e68]: Workspace
          - strong [ref=e69]: All local projects
        - group "Capture and egress status" [ref=e70]:
          - generic [ref=e71]: Proxy off
          - generic [ref=e73]: AI egress on confirmation only
      - main [ref=e74]:
        - generic [ref=e75]:
          - generic [ref=e77]:
            - paragraph [ref=e78]: Patch semantics · no hidden resets
            - heading "Settings" [level=1] [ref=e79]
            - paragraph [ref=e80]: Each control changes only its named field. Credentials live in the operating-system keychain, never in the settings file.
          - status [ref=e81]:
            - paragraph [ref=e84]: Appearance set to system.
          - generic [ref=e85]:
            - region [ref=e86]:
              - generic [ref=e92]:
                - paragraph [ref=e93]: Display
                - heading "Appearance" [level=2] [ref=e94]
              - generic [ref=e95]:
                - generic [ref=e96]:
                  - strong [ref=e97]: Color theme
                  - paragraph [ref=e98]: Follow your device, or keep Cutokyo light or dark. This choice affects only Cutokyo.
                - group "Appearance" [ref=e100]:
                  - button "Light" [ref=e101] [cursor=pointer]
                  - button "Dark" [ref=e102] [cursor=pointer]
                  - button "System" [active] [pressed] [ref=e103] [cursor=pointer]
            - region [ref=e104]:
              - generic [ref=e109]:
                - paragraph [ref=e110]: Local privacy
                - heading "Capture and agent access" [level=2] [ref=e111]
              - generic [ref=e112]:
                - generic [ref=e113]:
                  - strong [ref=e114]: Native capture
                  - paragraph [ref=e115]: Preview, install, recover, or remove Cutokyo-owned capture for each harness. Configuration verification is separate from live capture. Your saved preferences and history are preserved.
                - link "Manage native capture" [ref=e117] [cursor=pointer]:
                  - /url: "#/onboarding"
              - generic [ref=e118]:
                - generic [ref=e119]:
                  - strong [ref=e120]: Read-only Cutokyo search MCP
                  - paragraph [ref=e121]: Let configured agents query attributable history. It exposes no mutation tools and never injects context automatically.
                - generic [ref=e123] [cursor=pointer]:
                  - generic [ref=e124]: Enabled
                  - switch "Toggle read-only Cutokyo search MCP" [checked] [ref=e125]
              - generic [ref=e126]:
                - generic [ref=e127]:
                  - strong [ref=e128]: Proxy capture fallback
                  - paragraph [ref=e129]: Optional provider-traffic attribution. Review content, provider routing, and local redaction before enabling. Native capture remains the default.
                - generic [ref=e130]:
                  - generic [ref=e131]:
                    - generic [aria-hidden] [ref=e132]: ·
                    - text: Unavailable
                  - button "Enable proxy" [disabled] [ref=e133] [cursor=pointer]
                  - button "Review data handling" [ref=e134] [cursor=pointer]
              - paragraph [ref=e139]: "Proxy capture is unavailable: this desktop build has no provider-proxy listener. Native capture remains available. Provider requests are forwarded unchanged. Credentials and raw payloads are not retained in local proxy traces."
            - region [ref=e140]:
              - generic [ref=e145]:
                - paragraph [ref=e146]: Storage
                - heading "Retention default" [level=2] [ref=e147]
              - generic [ref=e148]:
                - generic [ref=e149]:
                  - strong [ref=e150]: Stored history
                  - paragraph [ref=e151]: A setting does not delete history by itself. Use Data controls to preview and apply the exact session set.
                - combobox "Default history retention" [ref=e153]:
                  - option "Keep until deleted" [selected]
                  - option "30 days"
                  - option "90 days"
                  - option "1 year"
              - paragraph [ref=e158]: The local database is plaintext with owner-only permissions in v0.x. Use full-disk encryption. Deleting rows is not physical secure erasure from WAL, SSDs, snapshots, or existing backups.
            - region [ref=e159]:
              - generic [ref=e166]:
                - paragraph [ref=e167]: Application
                - heading "Updates and crash records" [level=2] [ref=e168]
              - generic [ref=e169]:
                - generic [ref=e170]:
                  - strong [ref=e171]: Updater choice
                  - paragraph [ref=e172]: Signed updater manifests are separate from platform code signing and notarization.
                - combobox "Updater behavior" [ref=e174]:
                  - option "Download signed updates automatically"
                  - option "Notify before downloading" [selected]
                  - option "Check manually"
              - generic [ref=e175]:
                - generic [ref=e176]:
                  - strong [ref=e177]: Bounded crash records
                  - paragraph [ref=e178]: Offer a sanitized local crash record on next launch. Records are never uploaded automatically.
                - generic [ref=e180] [cursor=pointer]:
                  - generic [ref=e181]: Disabled
                  - switch "Toggle bounded local crash records" [ref=e182]
              - generic [ref=e183]:
                - generic [ref=e184]:
                  - strong [ref=e185]: Check for signed update
                  - paragraph [ref=e186]: A failed network check does not change the current installation.
                - button "Check now" [ref=e188] [cursor=pointer]
            - region [ref=e194]:
              - generic [ref=e199]:
                - paragraph [ref=e200]: Credentials
                - heading "OS keychain only" [level=2] [ref=e201]
              - paragraph [ref=e202]: Provider keys are never displayed here, written to TOML, or included in diagnostic bundles.
  - status [ref=e203]: Appearance set to system.
```

# Test source

```ts
  269 |   page,
  270 | }) => {
  271 |   await openCase(page, "visual-keyboard-consistency", "/dashboard", "Overview");
  272 |   await page.keyboard.press("Tab");
  273 |   const skip = page.getByRole("link", { name: "Skip to content" });
  274 |   await expect(skip).toBeFocused();
  275 |   await expect(skip).toHaveCSS("transform", "matrix(1, 0, 0, 1, 0, 0)");
  276 |   await page.keyboard.press("Tab");
  277 |   await expect(page.getByRole("link", { name: "Overview" })).toBeFocused();
  278 |   await page.keyboard.press("Tab");
  279 |   const sessionsNavigation = page
  280 |     .getByRole("navigation", { name: "Primary navigation" })
  281 |     .getByRole("link", { name: "Sessions", exact: true });
  282 |   await expect(sessionsNavigation).toBeFocused();
  283 |   await page.keyboard.press("Enter");
  284 |   const heading = page.getByRole("heading", { name: "Sessions", level: 1 });
  285 |   await expect(heading).toBeVisible();
  286 |   await expect(heading).toBeFocused();
  287 | });
  288 | 
  289 | test("dashboard and consent surfaces pass browser accessibility checks", async ({
  290 |   page,
  291 | }) => {
  292 |   await openCase(page, "populated-dashboard", "/dashboard", "Overview");
  293 |   const dashboard = await new AxeBuilder({ page }).analyze();
  294 |   expect(dashboard.violations).toEqual([]);
  295 | 
  296 |   await openCase(page, "analysis-preview-cancel", "/analysis", "AI analysis");
  297 |   const candidate = page
  298 |     .locator("article")
  299 |     .filter({ hasText: "analysis-73A9" });
  300 |   await candidate.getByRole("button", { name: "Analyze" }).click();
  301 |   const consentDialog = page.getByRole("dialog", {
  302 |     name: "Review AI analysis egress",
  303 |   });
  304 |   await expect(consentDialog).toBeVisible();
  305 |   await expect(
  306 |     consentDialog.getByRole("button", {
  307 |       name: "Confirm and send redacted payload",
  308 |     }),
  309 |   ).toBeEnabled();
  310 |   const consent = await new AxeBuilder({ page }).analyze();
  311 |   expect(consent.violations).toEqual([]);
  312 | });
  313 | 
  314 | test("System follows the OS; explicit appearance survives route navigation", async ({
  315 |   page,
  316 | }) => {
  317 |   await page.setViewportSize({ width: 900, height: 700 });
  318 |   await page.emulateMedia({ colorScheme: "dark" });
  319 |   await openCase(page, "visual-keyboard-consistency", "/settings", "Settings");
  320 |   await expectNoWindowOverflow(page);
  321 |   const root = page.locator("html");
  322 |   const color = page.locator('meta[name="theme-color"]');
  323 |   await expect(
  324 |     page.getByRole("button", { name: "System", pressed: true }),
  325 |   ).toBeVisible();
  326 |   await expect(root).toHaveAttribute("data-theme", "dark");
  327 |   await expect(color).toHaveAttribute("content", "#111411");
  328 | 
  329 |   await page.emulateMedia({ colorScheme: "light" });
  330 |   await expect(root).toHaveAttribute("data-theme", "light");
  331 |   await page.getByRole("button", { name: "Dark" }).click();
  332 |   await expect(
  333 |     page.getByRole("button", { name: "Dark", pressed: true }),
  334 |   ).toBeVisible();
  335 |   await expect(root).toHaveAttribute("data-theme", "dark");
  336 |   await expect(
  337 |     page.locator("#main-content").getByText("Appearance set to dark."),
  338 |   ).toBeVisible();
  339 |   expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  340 |   await page.emulateMedia({ colorScheme: "dark" });
  341 |   await page.emulateMedia({ colorScheme: "light" });
  342 |   await expect(root).toHaveAttribute("data-theme", "dark");
  343 | 
  344 |   await page.getByRole("link", { name: "Overview" }).click();
  345 |   await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  346 |   await page.getByRole("link", { name: "Settings" }).click();
  347 |   await expect(
  348 |     page.getByRole("button", { name: "Dark", pressed: true }),
  349 |   ).toBeVisible();
  350 |   const light = page.getByRole("button", { name: "Light" });
  351 |   await light.focus();
  352 |   await expect(light).toBeFocused();
  353 |   await page.keyboard.press("Space");
  354 |   await expect(
  355 |     page.getByRole("button", { name: "Light", pressed: true }),
  356 |   ).toBeVisible();
  357 |   await expect(
  358 |     page.locator("#main-content").getByText("Appearance set to light."),
  359 |   ).toBeVisible();
  360 |   await expect(light).toBeFocused();
  361 |   await expect(root).toHaveAttribute("data-theme", "light");
  362 |   await page.emulateMedia({ colorScheme: "dark" });
  363 |   await expect(root).toHaveAttribute("data-theme", "light");
  364 |   await page.getByRole("button", { name: "System" }).click();
  365 |   await expect(root).toHaveAttribute("data-theme", "dark");
  366 |   await page.emulateMedia({ colorScheme: "light" });
  367 |   await expect(root).toHaveAttribute("data-theme", "light");
  368 |   const a11y = await new AxeBuilder({ page }).analyze();
> 369 |   expect(a11y.violations).toEqual([]);
      |                           ^ Error: expect(received).toEqual(expected) // deep equality
  370 | });
  371 | 
  372 | test("saved dark appears before app render; failed save restores System", async ({
  373 |   page,
  374 | }) => {
  375 |   await page.addInitScript(() => {
  376 |     const watcher = new MutationObserver(() => {
  377 |       if (document.querySelector("#root > *") !== null) {
  378 |         (window as Window & { __firstAppTheme?: string }).__firstAppTheme =
  379 |           document.documentElement.dataset.theme ?? "unset";
  380 |         watcher.disconnect();
  381 |       }
  382 |     });
  383 |     watcher.observe(document, { childList: true, subtree: true });
  384 |   });
  385 |   await openCase(page, "appearance-dark", "/settings", "Settings");
  386 |   await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  387 |   expect(
  388 |     await page.evaluate(
  389 |       () => (window as Window & { __firstAppTheme?: string }).__firstAppTheme,
  390 |     ),
  391 |   ).toBe("dark");
  392 |   await expect(
  393 |     page.getByRole("button", { name: "Dark", pressed: true }),
  394 |   ).toBeVisible();
  395 | 
  396 |   await openCase(page, "appearance-save-error", "/settings", "Settings");
  397 |   await page.getByRole("button", { name: "Dark" }).click();
  398 |   await expect(page.getByRole("alert")).toContainText(
  399 |     "Could not save appearance",
  400 |   );
  401 |   await expect(
  402 |     page.getByRole("button", { name: "System", pressed: true }),
  403 |   ).toBeVisible();
  404 |   await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  405 | });
  406 | 
  407 | test("a concurrent appearance write reloads the saved winner instead of old Dark", async ({
  408 |   page,
  409 | }) => {
  410 |   await mkdir(DARK_SCREENSHOTS, { recursive: true });
  411 |   await page.setViewportSize({ width: 900, height: 700 });
  412 |   await openCase(page, "appearance-conflict", "/settings", "Settings");
  413 |   await expect(
  414 |     page.getByRole("button", { name: "Dark", pressed: true }),
  415 |   ).toBeVisible();
  416 |   await page.getByRole("button", { name: "Light" }).click();
  417 |   await expect(page.getByRole("alert")).toContainText(
  418 |     "Saved appearance is system; your light choice was not saved.",
  419 |   );
  420 |   await expect(
  421 |     page.getByRole("button", { name: "System", pressed: true }),
  422 |   ).toBeVisible();
  423 |   await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  424 |   await expectNoWindowOverflow(page);
  425 |   expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  426 |   await page.screenshot({
  427 |     path: resolve(DARK_SCREENSHOTS, "settings-conflict-900x700.png"),
  428 |   });
  429 | });
  430 | 
  431 | test("slow saved Dark prevents a wrong-theme app render past 500ms", async ({
  432 |   page,
  433 | }) => {
  434 |   await page.addInitScript(() => {
  435 |     const watcher = new MutationObserver(() => {
  436 |       if (document.querySelector("#root .app-shell") !== null) {
  437 |         (window as Window & { __firstAppTheme?: string }).__firstAppTheme =
  438 |           document.documentElement.dataset.theme ?? "unset";
  439 |         watcher.disconnect();
  440 |       }
  441 |     });
  442 |     watcher.observe(document, { childList: true, subtree: true });
  443 |   });
  444 |   await page.goto("/?jev_case=appearance-dark-delayed#/settings");
  445 |   await expect(
  446 |     page
  447 |       .getByRole("status")
  448 |       .filter({ hasText: "Waiting for your saved appearance" }),
  449 |   ).toBeVisible();
  450 |   await expect(page.getByRole("heading", { name: "Settings" })).toHaveCount(0);
  451 |   await expect(
  452 |     page.getByRole("button", { name: /Continue with device appearance/ }),
  453 |   ).toBeVisible();
  454 |   await page.evaluate(() => {
  455 |     const release = (
  456 |       window as Window & { __CUTOKYO_FIXTURE_RELEASE_APPEARANCE__?: () => void }
  457 |     ).__CUTOKYO_FIXTURE_RELEASE_APPEARANCE__;
  458 |     if (release === undefined)
  459 |       throw new Error("Appearance fixture gate is missing.");
  460 |     release();
  461 |   });
  462 |   await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  463 |   await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  464 |   expect(
  465 |     await page.evaluate(
  466 |       () => (window as Window & { __firstAppTheme?: string }).__firstAppTheme,
  467 |     ),
  468 |   ).toBe("dark");
  469 | });
```