// Markdown for the Ask panel's answers. Model output is untrusted: this parses it into plain data and renders that with
// a createElement you pass in (React's in the panel), never through innerHTML. Raw HTML stays visible text; a link is
// its text with the address shown, never an anchor; an image is its description and address, never an <img>, so no
// address in an answer is ever opened or requested. An unclosed code fence while streaming is a code block so far.
// Math between $ … $ and $$ … $$ is shown as written in a mono span. Pure: src/lib/markdown.test.mjs.

export type Inline =
  | { t: "text"; v: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "math"; v: string }
  | { t: "link"; c: Inline[]; url: string }
  | { t: "br" };

export type CodeBlock = { t: "code"; lang: string; v: string; open: boolean };
export type Block =
  | { t: "p"; c: Inline[] }
  | { t: "h"; level: number; c: Inline[] }
  | CodeBlock
  | { t: "math"; v: string }
  | { t: "list"; ordered: boolean; start: number; items: Inline[][] }
  | { t: "quote"; c: Inline[] }
  | { t: "hr" };

const NL = String.fromCharCode(10);
const CR = String.fromCharCode(13);
const TAB = String.fromCharCode(9);
const BS = String.fromCharCode(92);
const PUNCT = "!\"#$%&'()*+,-./:;<=>?@[]^_`{|}~" + BS;

const isSpace = (c: string | undefined) => c === " " || c === TAB || c === NL || c === undefined;
const isDigit = (c: string | undefined) => c !== undefined && c >= "0" && c <= "9";
const isWord = (c: string | undefined) => c !== undefined && (isDigit(c) || c.toLowerCase() !== c.toUpperCase());
const indentOf = (line: string) => {
  let n = 0;
  while (n < line.length && (line[n] === " " || line[n] === TAB)) n += line[n] === TAB ? 4 : 1;
  return n;
};

// —— Blocks ——

type Fence = { ch: string; n: number; info: string };
function fenceOf(line: string): Fence | null {
  if (indentOf(line) > 3) return null;
  const s = line.trimStart();
  const ch = s[0];
  if (ch !== "`" && ch !== "~") return null;
  let n = 0;
  while (s[n] === ch) n++;
  if (n < 3) return null;
  const info = s.slice(n).trim();
  if (ch === "`" && info.includes("`")) return null;
  return { ch, n, info };
}
const closesFence = (line: string, f: Fence) => {
  if (indentOf(line) > 3) return false;
  const s = line.trim();
  if (s.length < f.n) return false;
  for (const c of s) if (c !== f.ch) return false;
  return true;
};

function headingOf(line: string): { level: number; text: string } | null {
  if (indentOf(line) > 3) return null;
  const s = line.trimStart();
  let n = 0;
  while (s[n] === "#") n++;
  if (n < 1 || n > 6 || !isSpace(s[n])) return null;
  let text = s.slice(n).trim();
  // A closing run of # is decoration.
  let e = text.length;
  while (e > 0 && text[e - 1] === "#") e--;
  if (e === 0 || text[e - 1] === " ") text = text.slice(0, e).trim();
  return { level: n, text };
}

function isRule(line: string): boolean {
  if (indentOf(line) > 3) return false;
  const s = line.trim();
  const ch = s[0];
  if (ch !== "-" && ch !== "*" && ch !== "_") return false;
  let n = 0;
  for (const c of s) {
    if (c === ch) n++;
    else if (c !== " " && c !== TAB) return false;
  }
  return n >= 3;
}

type Marker = { ordered: boolean; start: number; indent: number; rest: string; ch: string };
function markerOf(line: string): Marker | null {
  const indent = indentOf(line);
  const s = line.trimStart();
  const c = s[0];
  if ((c === "-" || c === "*" || c === "+") && (s[1] === " " || s[1] === TAB || s.length === 1))
    return { ordered: false, start: 1, indent, rest: s.slice(2), ch: c };
  let n = 0;
  while (n < 9 && isDigit(s[n])) n++;
  if (n > 0 && (s[n] === "." || s[n] === ")") && (s[n + 1] === " " || s[n + 1] === TAB || s.length === n + 1))
    return { ordered: true, start: Number(s.slice(0, n)), indent, rest: s.slice(n + 2), ch: s[n] };
  return null;
}

const startsMath = (line: string) => indentOf(line) <= 3 && line.trim().startsWith("$$");
const isQuote = (line: string) => indentOf(line) <= 3 && line.trimStart().startsWith(">");
const stripQuote = (line: string) => {
  const s = line.trimStart().slice(1);
  return s.startsWith(" ") ? s.slice(1) : s;
};

/** Whether a line starts a block that ends a paragraph. */
function interrupts(line: string): boolean {
  if (fenceOf(line) || headingOf(line) || isRule(line) || startsMath(line) || isQuote(line)) return true;
  const m = markerOf(line);
  return !!m && m.rest.trim() !== "" && (!m.ordered || m.start === 1);
}

export function parseMarkdown(src: string): Block[] {
  const lines = src.split(NL).map((l) => (l.endsWith(CR) ? l.slice(0, -1) : l));
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i++;
      continue;
    }
    const f = fenceOf(line);
    if (f) {
      const body: string[] = [];
      const cut = Math.min(indentOf(line), 3);
      i++;
      let open = true;
      while (i < lines.length) {
        if (closesFence(lines[i], f)) {
          open = false;
          i++;
          break;
        }
        const l = lines[i];
        body.push(indentOf(l) >= cut ? l.slice(Math.min(cut, l.length - l.trimStart().length)) : l.trimStart());
        i++;
      }
      out.push({ t: "code", lang: f.info.split(" ")[0] ?? "", v: body.join(NL), open });
      continue;
    }
    if (startsMath(line)) {
      const first = line.trim().slice(2);
      const close = first.indexOf("$$");
      if (close >= 0) {
        out.push({ t: "math", v: first.slice(0, close).trim() });
        i++;
        continue;
      }
      const body: string[] = first.trim() ? [first] : [];
      i++;
      while (i < lines.length) {
        const at = lines[i].indexOf("$$");
        if (at >= 0) {
          if (lines[i].slice(0, at).trim()) body.push(lines[i].slice(0, at));
          i++;
          break;
        }
        body.push(lines[i]);
        i++;
      }
      out.push({ t: "math", v: body.join(NL).trim() });
      continue;
    }
    const h = headingOf(line);
    if (h) {
      out.push({ t: "h", level: h.level, c: parseInline(h.text) });
      i++;
      continue;
    }
    if (isRule(line)) {
      out.push({ t: "hr" });
      i++;
      continue;
    }
    if (isQuote(line)) {
      const body: string[] = [];
      while (i < lines.length && lines[i].trim() !== "" && (isQuote(lines[i]) || !interrupts(lines[i]))) {
        body.push(isQuote(lines[i]) ? stripQuote(lines[i]) : lines[i]);
        i++;
      }
      out.push({ t: "quote", c: parseInline(body.join(NL)) });
      continue;
    }
    const m = markerOf(line);
    if (m) {
      const items: string[][] = [];
      let cur: string[] = [m.rest];
      i++;
      while (i < lines.length) {
        const l = lines[i];
        if (l.trim() === "") {
          // A blank line ends the list unless the next item of the same kind follows.
          let j = i + 1;
          while (j < lines.length && lines[j].trim() === "") j++;
          const nx = j < lines.length ? markerOf(lines[j]) : null;
          if (nx && nx.ordered === m.ordered) {
            i = j;
            continue;
          }
          if (j < lines.length && indentOf(lines[j]) >= 2 && !markerOf(lines[j])) {
            cur.push(lines[j].trim());
            i = j + 1;
            continue;
          }
          break;
        }
        const nm = markerOf(l);
        if (nm && (nm.ordered === m.ordered || nm.indent >= 2)) {
          // Nested items are kept as items of this list: the panel is too narrow for indented trees.
          items.push(cur);
          cur = [nm.rest];
          i++;
          continue;
        }
        if (nm || fenceOf(l) || headingOf(l) || isRule(l) || startsMath(l) || isQuote(l)) break;
        cur.push(l.trim());
        i++;
      }
      items.push(cur);
      out.push({ t: "list", ordered: m.ordered, start: m.start, items: items.map((it) => parseInline(it.join(NL))) });
      continue;
    }
    const body: string[] = [line];
    i++;
    while (i < lines.length && lines[i].trim() !== "" && !interrupts(lines[i])) {
      body.push(lines[i]);
      i++;
    }
    out.push({ t: "p", c: parseInline(body.map((l) => l.trimStart()).join(NL)) });
  }
  return out;
}

// —— Inlines ——

/** The end of a run of `ch` starting at i. */
const runEnd = (s: string, i: number, ch: string) => {
  let j = i;
  while (s[j] === ch) j++;
  return j;
};

/** Where a code span opened by n backticks at i closes, or -1. */
function codeClose(s: string, from: number, n: number): number {
  let j = from;
  while (j < s.length) {
    if (s[j] === "`") {
      const e = runEnd(s, j, "`");
      if (e - j === n) return j;
      j = e;
    } else j++;
  }
  return -1;
}

/** Where emphasis of `delim` opened at i closes: a matching run preceded by a non-space, outside code spans. */
function emClose(s: string, from: number, delim: string): number {
  const ch = delim[0];
  let j = from;
  while (j < s.length) {
    const c = s[j];
    if (c === BS) {
      j += 2;
      continue;
    }
    if (c === "`") {
      const n = runEnd(s, j, "`") - j;
      const k = codeClose(s, j + n, n);
      j = k < 0 ? j + n : k + n;
      continue;
    }
    if (c === ch) {
      const e = runEnd(s, j, ch);
      const len = e - j;
      if (len >= delim.length && !isSpace(s[j - 1]) && j > from) {
        // Take the run's end when it is longer (***x*** closes as strong around em); for _ the next char is no word char.
        const at = len === delim.length ? j : delim.length === 1 ? e - 1 : j;
        if (ch !== "_" || !isWord(s[at + delim.length])) return at;
      }
      j = e;
      continue;
    }
    j++;
  }
  return -1;
}

/** A link at i ("[" or "!["): its text, address and end, or null. */
function linkAt(s: string, i: number): { text: string; url: string; end: number } | null {
  const open = s[i] === "!" ? i + 1 : i;
  if (s[open] !== "[") return null;
  let depth = 0;
  let j = open;
  for (; j < s.length; j++) {
    if (s[j] === BS) {
      j++;
      continue;
    }
    if (s[j] === "[") depth++;
    else if (s[j] === "]" && --depth === 0) break;
  }
  if (j >= s.length || s[j + 1] !== "(") return null;
  let k = j + 2;
  let par = 1;
  for (; k < s.length; k++) {
    if (s[k] === "(") par++;
    else if (s[k] === ")" && --par === 0) break;
    else if (s[k] === NL) return null;
  }
  if (k >= s.length) return null;
  let url = s.slice(j + 2, k).trim();
  // A title after the address ("…" or '…') is not shown.
  const sp = url.search(/ ["']/);
  if (sp > 0) url = url.slice(0, sp).trim();
  if (url.startsWith("<") && url.endsWith(">")) url = url.slice(1, -1);
  return { text: s.slice(open + 1, j), url, end: k + 1 };
}

/** <https://…> or <mailto:…>: the address as text. Anything else after < (raw HTML) stays text. */
function autolinkAt(s: string, i: number): { url: string; end: number } | null {
  const close = s.indexOf(">", i);
  if (close < 0) return null;
  const v = s.slice(i + 1, close);
  if (!/^(https?:|mailto:)/i.test(v)) return null;
  for (const c of v) if (isSpace(c) || c === "<") return null;
  return { url: v, end: close + 1 };
}

/** Inline math at i: $…$ with no space just inside and no digit right after (so "$5 and $10" stays money). */
function mathAt(s: string, i: number): { v: string; end: number } | null {
  if (s[i + 1] === "$") {
    const close = s.indexOf("$$", i + 2);
    return close > i + 2 ? { v: s.slice(i + 2, close).trim(), end: close + 2 } : null;
  }
  if (isSpace(s[i + 1])) return null;
  for (let j = i + 1; j < s.length; j++) {
    if (s[j] === NL) return null;
    if (s[j] === BS) {
      j++;
      continue;
    }
    if (s[j] === "$") {
      if (isSpace(s[j - 1]) || isDigit(s[j + 1]) || j === i + 1) return null;
      return { v: s.slice(i + 1, j), end: j + 1 };
    }
  }
  return null;
}

export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  const flush = () => {
    if (buf) out.push({ t: "text", v: buf });
    buf = "";
  };
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === BS && i + 1 < s.length && PUNCT.includes(s[i + 1])) {
      buf += s[i + 1];
      i += 2;
      continue;
    }
    if (c === NL) {
      // Two trailing spaces or a backslash end the line; a plain newline is a space.
      if (buf.endsWith("  ") || buf.endsWith(BS)) {
        buf = buf.replace(/[ ]+$/, "");
        if (buf.endsWith(BS)) buf = buf.slice(0, -1);
        flush();
        out.push({ t: "br" });
      } else buf += " ";
      i++;
      continue;
    }
    if (c === "`") {
      const n = runEnd(s, i, "`") - i;
      const k = codeClose(s, i + n, n);
      if (k < 0) {
        buf += s.slice(i, i + n);
        i += n;
        continue;
      }
      let v = s.slice(i + n, k).split(NL).join(" ");
      if (v.length > 2 && v.startsWith(" ") && v.endsWith(" ")) v = v.slice(1, -1);
      flush();
      out.push({ t: "code", v });
      i = k + n;
      continue;
    }
    if (c === "$") {
      const m = mathAt(s, i);
      if (m) {
        flush();
        out.push({ t: "math", v: m.v });
        i = m.end;
        continue;
      }
    }
    if (c === "[" || (c === "!" && s[i + 1] === "[")) {
      const l = linkAt(s, i);
      if (l) {
        flush();
        // An image is its description and its address, like a link: nothing is fetched.
        out.push({ t: "link", c: parseInline(l.text || (c === "!" ? "image" : l.url)), url: l.url });
        i = l.end;
        continue;
      }
    }
    if (c === "<") {
      const a = autolinkAt(s, i);
      if (a) {
        flush();
        out.push({ t: "link", c: [{ t: "text", v: a.url }], url: a.url });
        i = a.end;
        continue;
      }
    }
    if (c === "*" || c === "_") {
      const e = runEnd(s, i, c);
      const n = e - i;
      const opens = !isSpace(s[e]) && (c === "*" || !isWord(s[i - 1]));
      if (opens) {
        for (const len of n >= 3 ? [3, 2, 1] : n === 2 ? [2, 1] : [1]) {
          const delim = c.repeat(len);
          const k = emClose(s, i + len, delim);
          if (k < 0) continue;
          flush();
          const inner = parseInline(s.slice(i + len, k));
          const node: Inline =
            len === 3 ? { t: "strong", c: [{ t: "em", c: inner }] } : len === 2 ? { t: "strong", c: inner } : { t: "em", c: inner };
          if (n > len) buf += c.repeat(n - len);
          flush();
          out.push(node);
          i = k + len;
          break;
        }
        if (i < e) {
          buf += s.slice(i, e);
          i = e;
        }
        continue;
      }
      buf += s.slice(i, e);
      i = e;
      continue;
    }
    buf += c;
    i++;
  }
  flush();
  return out;
}

// —— Rendering ——

/** createElement's shape: React.createElement in the panel, a string builder in the tests. */
export type H<N> = (type: string, props: Record<string, unknown> | null, ...children: (N | string)[]) => N;

export type RenderOpts<N> = {
  /** Draws a fenced code block (the panel adds a Copy button). `tail` goes after the code's last character. */
  code?: (b: CodeBlock, key: string, tail: N | null) => N;
  /** Appended after the last character of the answer: the panel puts a marker there to find where the text ends. */
  tail?: N;
};

/** The elements this renderer can produce: never a, img, script, iframe, style or anything that loads. */
export const ELEMENTS = ["p", "strong", "em", "code", "pre", "span", "br", "ul", "ol", "li", "blockquote", "hr", "div"] as const;

function inl<N>(xs: Inline[], h: H<N>, k: string): (N | string)[] {
  return xs.map((x, i): N | string => {
    const key = `${k}.${i}`;
    switch (x.t) {
      case "text":
        return x.v;
      case "strong":
        return h("strong", { key }, ...inl(x.c, h, key));
      case "em":
        return h("em", { key }, ...inl(x.c, h, key));
      case "code":
        return h("code", { key }, x.v);
      case "math":
        return h("span", { key, className: "md-math" }, x.v);
      case "br":
        return h("br", { key });
      case "link": {
        const text = inl(x.c, h, key);
        const same = x.c.length === 1 && x.c[0].t === "text" && x.c[0].v === x.url;
        return same
          ? h("span", { key, className: "md-url" }, x.url)
          : h("span", { key, className: "md-link" }, ...text, " ", h("span", { className: "md-url" }, `(${x.url})`));
      }
    }
  });
}

export function renderMarkdown<N>(blocks: Block[], h: H<N>, o: RenderOpts<N> = {}): N[] {
  const tail = o.tail ?? null;
  return blocks.map((b, i) => {
    const key = `b${i}`;
    const last = i === blocks.length - 1;
    const end = (xs: (N | string)[]) => (last && tail != null ? [...xs, tail] : xs);
    switch (b.t) {
      case "p":
        return h("p", { key }, ...end(inl(b.c, h, key)));
      case "h":
        return h("p", { key, className: "md-h" }, ...end(inl(b.c, h, key)));
      case "quote":
        return h("blockquote", { key }, ...end(inl(b.c, h, key)));
      case "math":
        return h("div", { key, className: "md-math md-math-block" }, ...end([b.v]));
      case "hr":
        return h("hr", { key });
      case "code":
        return o.code ? o.code(b, key, last ? tail : null) : h("pre", { key }, h("code", null, ...end([b.v])));
      case "list":
        return h(
          b.ordered ? "ol" : "ul",
          b.ordered && b.start !== 1 ? { key, start: b.start } : { key },
          ...b.items.map((it, j) => {
            const k = `${key}.${j}`;
            const xs = inl(it, h, k);
            return h("li", { key: k }, ...(j === b.items.length - 1 ? end(xs) : xs));
          }),
        );
    }
  });
}
