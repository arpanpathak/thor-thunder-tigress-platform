// Attachments, formulas and diagrams, against a local static preview with
// deterministic API fixtures. PLAYWRIGHT_MODULE and CHROME_PATH may point to an
// existing local installation; BROWSER=webkit runs it as Safari would.
import assert from 'node:assert/strict';
const engines = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const browser = await engines[process.env.BROWSER || 'chromium'].launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) });
const origin = process.env.CHAT_PREVIEW || 'http://127.0.0.1:8765';
const page = await browser.newPage({ viewport: { width: 1180, height: 820 } });
const errors = [];
const requests = [];
page.on('pageerror', (error) => errors.push(error.message));

// A 4×3 PNG, and a one-page PDF whose text layer says PELICAN.
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEElEQVR4nGO4IyICRww4OQD3Lwwx0Tw4hAAAAABJRU5ErkJggg==', 'base64');
const stream = 'BT /F1 18 Tf 72 720 Td (The secret word is PELICAN.) Tj ET';
const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
  `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`, '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
let pdf = '%PDF-1.4\n';
const offsets = objects.map((object, index) => { const at = pdf.length; pdf += `${index + 1} 0 obj\n${object}\nendobj\n`; return at; });
const xref = pdf.length;
pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((at) => `${String(at).padStart(10, '0')} 00000 n \n`).join('')}`;
pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;

await page.route('**/v1/models', (route) => route.fulfill({ json: { data: [
  { id: 'Seeing-Test', owned_by: 'llamacpp', architecture: { input_modalities: ['text', 'image'] } },
  { id: 'Text-Test', owned_by: 'llamacpp', architecture: { input_modalities: ['text'] } },
] } }));
await page.route('**/v1/chat/completions', async (route) => {
  requests.push(route.request().postDataJSON());
  await route.fulfill({ contentType: 'text/event-stream', body: 'data: ' + JSON.stringify({ choices: [{ delta: { content: 'Seen: $a^2+b^2=c^2$\n\n```mermaid\nflowchart LR\n  A[Pi] --> B[Thor]\n```' }, finish_reason: 'stop' }] }) + '\n\ndata: [DONE]\n\n' });
});

const until = (test, ms = 30000) => page.waitForFunction(test, null, { timeout: ms });

try {
  await page.goto(origin);
  await page.waitForSelector('#model option[value]', { state: 'attached' });

  // Markdown: formulas, prices, figures, and a drawing's script never runs.
  const report = await page.evaluate(async () => {
    const { markdown } = await import('./markdown.js');
    const { enrich } = await import('./rich.js');
    const box = document.createElement('div');
    document.body.append(box);
    markdown(box, ['Euler $e^{i\\pi}+1=0$, it costs $5 and $10.', '', '$$', '\\frac{a}{b}', '$$', '', 'Inline \\(x_1\\) and \\[ y^2 \\]', '',
      '```mermaid', 'flowchart LR', '  A --> B', '```', '', '```svg', '<svg viewBox="0 0 4 4"><rect width="4" height="4"/><script>parent.HACKED=1</script></svg>', '```', '',
      '```mermaid', 'not a diagram ((', '```', '', '```mermaid', 'flowchart TD'].join('\n'));
    enrich(box);
    for (let i = 0; i < 150 && (box.querySelectorAll('.math.typeset').length < 4 || box.querySelectorAll('.figure.drawn, .figure.failed').length < 3); i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    const result = {
      typeset: box.querySelectorAll('.math.typeset').length,
      prices: box.querySelector('p').textContent.includes('costs $5 and $10'),
      figures: [...box.querySelectorAll('.figure')].map((figure) => `${figure.dataset.kind}:${figure.classList.contains('drawn') ? 'drawn' : 'failed'}`),
      openFence: box.lastElementChild.matches('.code:not(.figure)'),
      hacked: Boolean(window.HACKED),
    };
    box.remove();
    return result;
  });
  assert.equal(report.typeset, 4, 'formulas typeset');
  assert.ok(report.prices, 'prices are not formulas');
  assert.deepEqual(report.figures, ['mermaid:drawn', 'svg:drawn', 'mermaid:failed']);
  assert.ok(report.openFence, 'a fence still streaming is code');
  assert.equal(report.hacked, false, 'a script in a drawing never runs');

  // Files: an image, a PDF and a refused binary; the request carries parts.
  await page.setInputFiles('#file-input', [
    { name: 'dot.png', mimeType: 'image/png', buffer: png },
    { name: 'secret.pdf', mimeType: 'application/pdf', buffer: Buffer.from(pdf, 'latin1') },
    { name: 'blob.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(64) },
  ]);
  await until(() => document.querySelectorAll('#attachments .attachment').length === 3 && !document.querySelector('#attachments .reading'));
  assert.equal(await page.locator('#attachments .failed').count(), 1, 'the binary file is refused');
  assert.match(await page.locator('#attachments .attachment').nth(1).innerText(), /1 page/);
  assert.equal(await page.locator('#attach-note').isHidden(), true);

  await page.selectOption('#model', 'Text-Test');
  await until(() => !document.querySelector('#attach-note').hidden);
  assert.match(await page.locator('#attach-note').innerText(), /can't see pictures.*Seeing Test can/s);
  await page.selectOption('#model', 'Seeing-Test');

  await page.fill('#input', 'What is in these?');
  await page.click('#send');
  await until(() => document.querySelector('.msg.assistant .stats'));
  const sent = requests.at(-1).messages.at(-1);
  assert.equal(sent.role, 'user');
  assert.equal(sent.content[0].type, 'image_url');
  assert.match(sent.content[0].image_url.url, /^data:image\/png;base64,/);
  assert.match(sent.content[1].text, /<file name="secret.pdf" pages="1">[\s\S]*PELICAN[\s\S]*What is in these\?$/);
  assert.equal(await page.locator('#attachments .attachment').count(), 0, 'sent files leave the composer');
  assert.equal(await page.locator('.msg.user .picture img').count(), 1);
  await until(() => document.querySelector('.msg.assistant .math.typeset') && document.querySelector('.msg.assistant .figure.drawn img'));

  // The same files with a model that can't see: the picture becomes a note.
  await page.selectOption('#model', 'Text-Test');
  await page.fill('#input', 'And now?');
  await page.click('#send');
  await until(() => document.querySelectorAll('.msg.assistant .stats').length === 2);
  const blind = requests.at(-1).messages.find((message) => Array.isArray(message.content));
  assert.ok(blind.content.every((part) => part.type === 'text'));
  assert.match(blind.content[0].text, /\[dot\.png: an image the current model can't see\]/);

  // After a reload the picture still opens full size, from IndexedDB.
  await page.reload();
  await page.locator('.msg.user .picture').first().click();
  await page.waitForSelector('#viewer[open] img');
  assert.deepEqual(await page.locator('#viewer img').evaluate((image) => [image.naturalWidth, image.naturalHeight]), [4, 3]);
  await page.keyboard.press('Escape');

  assert.deepEqual(errors, []);
  console.log('PASS: formulas, prices kept as text, diagrams, sandboxed drawings, image/PDF/text attachments, refused binaries, text-only models, reload, no browser errors');
} finally {
  await browser.close();
}
