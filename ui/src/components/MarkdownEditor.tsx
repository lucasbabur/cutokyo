import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { Annotation, EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { Save } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { liveMarkdown } from "./markdownLive.js";
import { Button, Modal } from "./Primitives.js";

type View = "write" | "split" | "preview";
const VIEWS: readonly { readonly id: View; readonly label: string }[] = [
  { id: "write", label: "Write" },
  { id: "split", label: "Split" },
  { id: "preview", label: "Preview" },
];
/** Marks document replacements that come from the parent, not from typing. */
const external = Annotation.define<boolean>();

/** Colours only; sizes and weights come from the live-rendering line classes. */
const highlight = HighlightStyle.define([
  { tag: [tags.link, tags.url], color: "var(--link)" },
  {
    tag: [tags.processingInstruction, tags.contentSeparator, tags.meta],
    color: "var(--text-muted)",
  },
  {
    tag: [tags.propertyName, tags.definition(tags.propertyName)],
    color: "var(--text-muted)",
  },
]);

const theme = EditorView.theme({
  "&": { height: "100%", color: "var(--text-primary)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { overflow: "auto", fontFamily: "inherit" },
  ".cm-content": {
    maxWidth: "720px",
    margin: "0 auto",
    padding: "48px 40px 40vh",
    fontSize: "var(--font-size-lg)",
    lineHeight: "1.75",
    caretColor: "var(--text-primary)",
  },
  ".cm-cursor": {
    borderLeftColor: "var(--text-primary)",
    borderLeftWidth: "2px",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection":
    { background: "var(--selection)" },
  ".cm-activeLine": { background: "transparent" },
});

/** Frontmatter is shown as a small field list above the rendered body. */
function splitFrontmatter(text: string) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (match === null) return { fields: [], body: text };
  const fields = (match[1] ?? "")
    .split(/\r?\n/)
    .map((line) => /^([\w-]+):\s*(.*)$/.exec(line))
    .filter((field) => field !== null)
    .map((field) => [field[1]!, field[2]!] as const);
  return { fields, body: text.slice(match[0].length) };
}

// Previews stay local: images show their alt text and links never navigate the app.
const previewComponents: Components = {
  img: ({ alt, src }) => (
    <span className="md-preview__image" title={src}>
      {alt === undefined || alt === "" ? "Image" : alt}
    </span>
  ),
  a: ({ children, href }) => (
    <span className="md-preview__link" title={href}>
      {children}
    </span>
  ),
};

const words = (text: string) =>
  text.split(/\s+/).filter((word) => /[\p{L}\p{N}]/u.test(word)).length;

/**
 * Counts what an agent would load. The tokenizer is OpenAI's o200k, so the count
 * is an estimate for Claude; it loads only when the editor opens.
 */
function useTokenEstimate(text: string): number | null {
  const [tokens, setTokens] = useState<number | null>(null);
  useEffect(() => {
    let current = true;
    const timer = globalThis.setTimeout(() => {
      void import("gpt-tokenizer").then(({ countTokens }) => {
        if (!current) return;
        try {
          setTokens(countTokens(text));
        } catch {
          setTokens(null);
        }
      });
    }, 250);
    return () => {
      current = false;
      globalThis.clearTimeout(timer);
    };
  }, [text]);
  return tokens;
}

const count = new Intl.NumberFormat();

/**
 * Full-window Markdown editor: live-rendered writing beside a rendered preview.
 * The parent owns the draft, saving and the dirty state.
 */
export function MarkdownEditor({
  title,
  path,
  value,
  readOnly,
  dirty,
  saving,
  error,
  onChange,
  onSave,
  onClose,
}: {
  readonly title: string;
  readonly path: string;
  readonly value: string;
  readonly readOnly: boolean;
  readonly dirty: boolean;
  readonly saving: boolean;
  readonly error: string | null;
  readonly onChange: (value: string) => void;
  readonly onSave: () => void;
  readonly onClose: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<EditorView | null>(null);
  const latest = useRef({ onChange, onSave, dirty, saving });
  latest.current = { onChange, onSave, dirty, saving };
  const [view, setView] = useState<View>("split");
  const tokens = useTokenEstimate(value);

  useEffect(() => {
    if (host.current === null) return;
    const instance = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: value,
        extensions: [
          history(),
          keymap.of([
            {
              key: "Mod-s",
              preventDefault: true,
              run: () => {
                const { dirty, saving, onSave } = latest.current;
                if (dirty && !saving) onSave();
                return true;
              },
            },
            ...defaultKeymap,
            ...historyKeymap,
          ]),
          // Skills open with YAML frontmatter; parse it as YAML, not as a setext heading.
          // The GitHub-flavoured base adds strikethrough, tables and task lists.
          yamlFrontmatter({ content: markdown({ base: markdownLanguage }) }),
          syntaxHighlighting(highlight),
          liveMarkdown,
          EditorView.lineWrapping,
          theme,
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
          EditorView.contentAttributes.of({ "aria-label": "Markdown source" }),
          EditorView.updateListener.of((update) => {
            const synced = update.transactions.some((transaction) =>
              transaction.annotation(external),
            );
            if (update.docChanged && !synced) {
              latest.current.onChange(update.state.doc.toString());
            }
          }),
        ],
      }),
    });
    editor.current = instance;
    instance.focus();
    return () => {
      instance.destroy();
      editor.current = null;
    };
    // The document is seeded once; later outside changes are synced below.
  }, [readOnly]);

  // Reverting or reloading replaces the document without recreating the editor.
  useEffect(() => {
    const instance = editor.current;
    if (instance === null || instance.state.doc.toString() === value) return;
    instance.dispatch({
      changes: { from: 0, to: instance.state.doc.length, insert: value },
      annotations: external.of(true),
    });
  }, [value]);

  const { fields, body } = splitFrontmatter(value);
  return (
    <Modal
      title={title}
      description={path}
      size="full"
      closeLabel="Close editor"
      closeDisabled={saving}
      onClose={onClose}
      actions={
        <>
          <div className="segmented" role="group" aria-label="Editor view">
            {VIEWS.map((entry) => (
              <button
                type="button"
                key={entry.id}
                className="segmented__item"
                aria-pressed={view === entry.id}
                onClick={() => setView(entry.id)}
              >
                {entry.label}
              </button>
            ))}
          </div>
          {readOnly ? null : (
            <Button
              size="small"
              variant="primary"
              disabled={!dirty || saving}
              onClick={onSave}
            >
              <Save aria-hidden="true" /> {saving ? "Saving…" : "Save"}
            </Button>
          )}
        </>
      }
    >
      {error === null ? null : (
        <p className="form-error md-editor__error" role="alert">
          {error}
        </p>
      )}
      <div className={`md-editor md-editor--${view}`}>
        <div className="md-editor__write" ref={host} />
        <article className="md-preview" aria-label="Preview">
          {fields.length === 0 ? null : (
            <dl className="md-preview__fields">
              {fields.map(([key, field]) => (
                <div key={key}>
                  <dt>{key}</dt>
                  <dd>{field}</dd>
                </div>
              ))}
            </dl>
          )}
          <Markdown remarkPlugins={[remarkGfm]} components={previewComponents}>
            {body}
          </Markdown>
        </article>
      </div>
      <footer className="md-editor__bar">
        <span>{count.format(words(body))} words</span>
        <span title="Estimated with the o200k tokenizer; Claude's count differs slightly.">
          {tokens === null
            ? "Counting tokens…"
            : `≈ ${count.format(tokens)} tokens`}
        </span>
        <span className="md-editor__state" role="status">
          {readOnly
            ? "Read-only"
            : saving
              ? "Saving…"
              : dirty
                ? "Unsaved changes"
                : "Saved"}
        </span>
        {readOnly ? null : <kbd>Ctrl S</kbd>}
      </footer>
    </Modal>
  );
}
