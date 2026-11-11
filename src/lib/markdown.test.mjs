// Run: node --test src/lib/markdown.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { ELEMENTS, parseMarkdown, renderMarkdown, settleTail } from "./markdown.ts";
import { PREVIEW_ANSWER } from "./live.ts";

const NL = String.fromCharCode(10);
const esc = (s) => s.split("&").join("&amp;").split("<").join("&lt;").split(">").join("&gt;");
let used = new Set();
const h = (type, props, ...children) => {
  used.add(type);
  const attrs = props
    ? Object.entries(props)
        .filter(([k]) => k !== "key")
        .map(([k, v]) => ` ${k}="${esc(String(v))}"`)
        .join("")
    : "";
  return { html: `<${type}${attrs}>${children.map((c) => (typeof c === "string" ? esc(c) : c.html)).join("")}</${type}>` };
};
const html = (src, o) => renderMarkdown(parseMarkdown(src), h, o).map((n) => n.html).join("");
const lines = (...xs) => xs.join(NL);

test("paragraphs, bold, italic and inline code", () => {
  assert.equal(
    html(lines("One **bold** and *it* and `x < y`.", "", "Two _also_ __strong__")),
    '<p>One <strong>bold</strong> and <em>it</em> and <code>x &lt; y</code>.</p><p>Two <em>also</em> <strong>strong</strong></p>',
  );
});

test("a plain newline inside a paragraph is a space; two trailing spaces break the line", () => {
  assert.equal(html(lines("a", "b")), "<p>a b</p>");
  assert.equal(html(lines("a  ", "b")), "<p>a<br></br>b</p>");
});

test("snake_case and arithmetic stay as written", () => {
  assert.equal(html("use snake_case_name and 2 * 3 * 4"), "<p>use snake_case_name and 2 * 3 * 4</p>");
});

test("a fenced code block keeps its text and language", () => {
  const b = parseMarkdown(lines("```python", "x = 1  # <b>", "```", "after"));
  assert.deepEqual(b[0], { t: "code", lang: "python", v: "x = 1  # <b>", open: false });
  assert.equal(b[1].t, "p");
  assert.equal(html(lines("```", "<b>hi</b>", "```")), "<pre><code>&lt;b&gt;hi&lt;/b&gt;</code></pre>");
});

test("an unclosed fence while streaming renders as a code block so far", () => {
  const b = parseMarkdown(lines("Here:", "", "```js", "const a = 1;", "const b"));
  assert.equal(b.length, 2);
  assert.deepEqual(b[1], { t: "code", lang: "js", v: lines("const a = 1;", "const b"), open: true });
  assert.equal(parseMarkdown("```").length, 1);
});

test("ordered and unordered lists, with the start number kept", () => {
  assert.equal(html(lines("- one", "- two **b**")), "<ul><li>one</li><li>two <strong>b</strong></li></ul>");
  assert.equal(html(lines("3. three", "4. four")), '<ol start="3"><li>three</li><li>four</li></ol>');
  assert.equal(html(lines("1. a", "", "2. b")), "<ol><li>a</li><li>b</li></ol>");
  assert.equal(html(lines("Intro:", "- x")), "<p>Intro:</p><ul><li>x</li></ul>");
});

test("raw HTML from the model is visible text, never an element", () => {
  used = new Set();
  const out = html(lines('<img src="https://evil.example/x.png" onerror="alert(1)">', "", "<script>alert(1)</script> <a href='https://x'>x</a>"));
  assert.ok(out.includes("&lt;img src="), out);
  assert.ok(out.includes("&lt;script&gt;"), out);
  for (const bad of ["img", "script", "a", "iframe", "style"]) assert.ok(!used.has(bad), bad);
});

test("a link is its text and its address, never an anchor that can open or fetch", () => {
  used = new Set();
  const out = html("See [the docs](https://example.com/a?b=1 \"Title\") or <https://example.org>.");
  assert.equal(
    out,
    '<p>See <span className="md-link">the docs <span className="md-url">(https://example.com/a?b=1)</span></span> or <span className="md-url">https://example.org</span>.</p>',
  );
  assert.ok(!used.has("a"));
});

test("an image is never requested: no <img> with a remote address can be produced", () => {
  used = new Set();
  const out = html("![a chart](https://evil.example/c.png) and ![](http://evil.example/d.png)");
  assert.ok(!used.has("img"));
  assert.ok(!out.includes("<img"));
  assert.ok(out.includes("a chart") && out.includes("(https://evil.example/c.png)"));
  assert.ok(out.includes("(http://evil.example/d.png)"));
});

test("math is shown as written in a mono span; money is not math", () => {
  assert.equal(html("Area $a^2 + b$ here"), '<p>Area <span className="md-math">a^2 + b</span> here</p>');
  assert.equal(html("$$P(B) = 1$$"), '<div className="md-math md-math-block">P(B) = 1</div>');
  assert.equal(html(lines("$$", "x = 1", "$$")), '<div className="md-math md-math-block">x = 1</div>');
  assert.equal(html("It costs $5 and $10."), "<p>It costs $5 and $10.</p>");
});

test("headings, quotes and rules are plain and quiet", () => {
  assert.equal(html("## Why"), '<p className="md-h">Why</p>');
  assert.equal(html("> quoted"), "<blockquote>quoted</blockquote>");
  assert.equal(html(lines("a", "", "---", "", "b")), "<p>a</p><hr></hr><p>b</p>");
});

test("the end marker goes after the last character, inside the last block (the caret's place)", () => {
  const tail = { html: "<i/>" };
  assert.equal(html(lines("a", "", "b"), { tail }), "<p>a</p><p>b<i/></p>");
  assert.equal(html(lines("- x", "- y"), { tail }), "<ul><li>x</li><li>y<i/></li></ul>");
  let got = null;
  html(lines("```", "code"), { tail, code: (b, key, t) => ((got = t), { html: "<pre/>" }) });
  assert.equal(got, tail);
});

test("every prefix of a streamed answer renders, only with the allowed elements", () => {
  const src = PREVIEW_ANSWER + NL + lines("", "<b>raw</b> **open", "```", "unclosed");
  for (let n = 0; n <= src.length; n++) {
    used = new Set();
    html(src.slice(0, n), { tail: { html: "" } });
    for (const t of used) assert.ok(ELEMENTS.includes(t), `${t} at ${n}`);
  }
});

test("while streaming, a line cut mid-syntax shows words, not markup; code is left as it came", () => {
  assert.equal(settleTail("More in [Bayes"), "More in Bayes");
  assert.equal(settleTail("More in [Bayes](https://en.wiki"), "More in Bayes");
  assert.equal(settleTail("See [a](b) done"), "See [a](b) done");
  assert.equal(html(settleTail("a **bol")), "<p>a <strong>bol</strong></p>");
  assert.equal(html(settleTail("call `fo")), "<p>call <code>fo</code></p>");
  assert.equal(settleTail("2 * 3"), "2 * 3");
  const code = lines("```", "x = [1, 2", "**y");
  assert.equal(settleTail(code), code);
  assert.equal(settleTail(lines("**done**", "next [x")), lines("**done**", "next x"));
});

test("parsing is deterministic", () => {
  assert.deepEqual(parseMarkdown(PREVIEW_ANSWER), parseMarkdown(PREVIEW_ANSWER));
});
