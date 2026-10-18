// Run against a local static preview. API responses are deterministic fixtures.
// PLAYWRIGHT_MODULE and CHROME_PATH may point to an existing local installation.
import assert from 'node:assert/strict';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) });
const origin = process.env.CHAT_PREVIEW || 'http://127.0.0.1:8765';
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [];
const requests = [];
page.on('pageerror', (error) => errors.push(error.message));
await page.route('**/v1/models', (route) => route.fulfill({ json: { data: [{ id: 'Nemotron-Test', owned_by: 'llamacpp' }] } }));
await page.route('**/v1/chat/completions', async (route) => {
  requests.push(route.request().postDataJSON());
  await route.fulfill({ contentType: 'text/event-stream', body: 'data: ' + JSON.stringify({ choices: [{ delta: { content: '## A clear answer\n\nFirst logical group.\nA deliberate line break.\n\nSecond logical group.\n\n- One point\n- Another point\n\n```rust\nfn main() {\n    println!("Hello");\n}\n```' }, finish_reason: 'stop' }] }) + '\n\ndata: [DONE]\n\n' });
});
try {
  await page.goto(origin);
  await page.waitForSelector('.empty h1');
  assert.ok((await page.locator('.cub-stage > svg').boundingBox()).width >= 300);
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-desktop.png' });
  for (const width of [320, 390, 768]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `Welcome overflow at ${width}px`);
    if (width === 390) {
      assert.ok((await page.locator('.cub-stage > svg').boundingBox()).width >= 220);
      await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-welcome-mobile.png' });
    }
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.locator('#open-history').click();
  assert.equal(await page.locator('#sidebar').isVisible(), false);
  assert.equal(await page.locator('#open-history').getAttribute('aria-expanded'), 'false');
  await page.reload();
  await page.waitForSelector('.empty h1');
  assert.equal(await page.locator('#sidebar').isVisible(), false);
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-collapsed.png' });
  await page.locator('#open-history').click();
  assert.equal(await page.locator('#sidebar').isVisible(), true);
  await page.locator('#close-history').click();
  assert.equal(await page.locator('#sidebar').isVisible(), false);
  await page.locator('#open-history').click();
  await page.locator('#open-themes').click();
  assert.equal(await page.locator('.theme-choice').count(), 20);
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-themes.png' });
  for (const theme of await page.locator('.theme-choice').evaluateAll((nodes) => nodes.map((node) => node.dataset.themeChoice))) {
    await page.locator(`[data-theme-choice="${theme}"]`).click();
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), theme);
    assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme), 'dark');
  }
  await page.locator('[data-theme-choice="graphite"]').click();
  await page.locator('#themes [data-filter="light"]').click();
  assert.equal(await page.locator('.theme-choice').count(), 3);
  await page.locator('#themes [data-filter="all"]').click();
  assert.equal(await page.locator('.theme-choice').count(), 24);
  await page.getByRole('button', { name: 'Close themes', exact: true }).click();
  await page.reload();
  await page.waitForSelector('.empty h1');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'graphite');
  await page.locator('#input').fill('First line');
  await page.locator('#input').press('Enter');
  await page.locator('#input').press('End');
  await page.locator('#input').press('a');
  assert.equal(await page.locator('#input').inputValue(), 'First line\na');
  assert.equal(requests.length, 0);
  await page.locator('#input').press('Shift+Enter');
  await page.waitForSelector('.msg.assistant .stats');
  assert.equal(requests.length, 1);
  assert.equal(requests[0].messages.at(-1).content, 'First line\na');
  assert.equal(await page.locator('.msg.assistant p').count(), 2);
  assert.equal(await page.locator('.msg.assistant pre').count(), 1);
  await page.reload();
  await page.waitForSelector('.msg.assistant .stats');
  assert.equal(await page.locator('.msg.user').textContent(), 'First line\na');
  await page.locator('#input').fill('Unsent\n\ndraft');
  await page.waitForTimeout(400);
  await page.locator('#new').click();
  await page.locator('#input').fill('Second conversation');
  await page.locator('#send').click();
  await page.waitForSelector('.msg.assistant .stats');
  assert.equal(await page.locator('.history-item').count(), 2);
  await page.locator('.history-item').filter({ hasText: 'First line' }).click();
  assert.equal(await page.locator('#input').inputValue(), 'Unsent\n\ndraft');
  const download = page.waitForEvent('download');
  await page.locator('#export-chat').click();
  assert.match((await download).suggestedFilename(), /thor-chat.*\.md/);
  await page.getByRole('button', { name: 'Delete Second conversation', exact: true }).click();
  await page.locator('#delete-chat button[value="delete"]').click();
  await page.waitForFunction(() => document.querySelectorAll('.history-item').length === 1);
  await page.locator('#input').fill('');
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-response.png' });
  for (const width of [320, 390, 768]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `Overflow at ${width}px`);
    const send = await page.locator('#send').boundingBox();
    assert.ok(send.x >= 0 && send.x + send.width <= width && send.y + send.height <= 844);
    await page.locator('#open-history').click();
    await page.waitForTimeout(250);
    assert.equal(await page.locator('#open-history').getAttribute('aria-expanded'), 'true');
    await page.locator('#close-history').click();
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(250);
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-mobile.png' });
  await page.locator('#open-themes').click();
  await page.locator('[data-theme-choice="deep-ocean"]').click();
  await page.getByRole('button', { name: 'Close themes', exact: true }).click();
  await page.screenshot({ animations: "disabled", path: '/tmp/thor-chat-mobile-dark.png' });
  await page.locator('#open-privacy').click();
  assert.match(await page.locator('#privacy').textContent(), /does not save your chat history/);
  await page.locator('#privacy button').click();
  const fixture = async (body) => page.route('**/v1/chat/completions', (route) => route.fulfill({ contentType: 'text/event-stream', body }));
  const ask = async (text) => {
    await page.locator('#input').fill(text);
    await page.locator('#send').click();
    await page.waitForFunction(() => document.querySelector('#scroller').getAttribute('aria-busy') === 'false');
  };
  await fixture('data: {"choices":[{"delta":{"content":"   "},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
  await ask('Blank response');
  assert.match(await page.locator('.msg.assistant').last().textContent(), /returned no answer/);
  await fixture('data: {"choices":[{"delta":{"reasoning":"A short thought"}}]}\n\ndata: [DONE]\n\n');
  await ask('Reasoning response');
  assert.match(await page.locator('.msg.assistant').last().textContent(), /A short thought/);
  assert.match(await page.locator('.msg.assistant').last().textContent(), /thought but wrote no answer/);
  await fixture('data: {"choices":[{"delta":{"content":"Partial answer"}}]}\n\n');
  await ask('Truncated response');
  assert.match(await page.locator('.msg.assistant').last().textContent(), /ended before completion/);
  await fixture('data: {"error":{"message":"Engine failed mid-stream"}}\n\n');
  await ask('Stream error');
  assert.match(await page.locator('.msg.assistant').last().textContent(), /Engine failed mid-stream/);
  await page.route('**/v1/chat/completions', async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.fulfill({ status: 503, body: 'Model unavailable' }).catch(() => {});
  });
  await page.locator('#input').fill('Test stopping');
  await page.locator('#send').click();
  assert.equal(await page.locator('#new').isDisabled(), true);
  await page.locator('#open-themes').click();
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#themes').isVisible(), false);
  assert.equal(await page.locator('#scroller').getAttribute('aria-busy'), 'true');
  await page.locator('#send').click();
  await page.waitForFunction(() => document.querySelector('#send').textContent.includes('Send'));
  assert.match(await page.locator('.msg.assistant').last().textContent(), /stopped/);
  await page.locator('#input').fill('Test an error');
  await page.locator('#send').click();
  await page.waitForFunction(() => document.querySelector('#scroller').getAttribute('aria-busy') === 'false');
  assert.match(await page.locator('.msg.assistant').last().textContent(), /503/);
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('Quota'); }; });
  await page.locator('#input').fill('Keep this draft');
  await page.waitForSelector('#storage-status:not([hidden])');
  assert.match(await page.locator('#storage-status').textContent(), /could not be saved locally/);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.locator('#new').click();
  assert.equal(await page.locator('#input').inputValue(), 'Keep this draft');
  await page.route('**/v1/models', (route) => route.fulfill({ status: 401, json: { error: 'Key required' } }));
  await page.reload();
  await page.waitForSelector('.gate');
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok((await page.locator('.gate .hero svg').boundingBox()).width >= 210);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.screenshot({ animations: 'disabled', path: '/tmp/thor-chat-invite-mobile.png' });
  assert.deepEqual(errors, []);
  console.log('PASS: persistent desktop sidebar collapse, mobile drawer, 24 themes (20 dark), theme persistence, keyboard, request formatting, markdown, reload, drafts, history switching, deletion, export, mobile overflow, themes, privacy, no browser errors');
} finally {
  await browser.close();
}
