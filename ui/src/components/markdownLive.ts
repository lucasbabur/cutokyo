import { syntaxTree } from "@codemirror/language";
import type { EditorState, Range } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";

/**
 * Obsidian-style live rendering: Markdown reads as formatted text, and its marks
 * reappear only on the lines being edited. Decorations change how the text looks,
 * never the text itself, so the saved file is exactly what was typed.
 */

class BulletWidget extends WidgetType {
  override eq() {
    return true;
  }
  override toDOM() {
    const bullet = document.createElement("span");
    bullet.className = "cm-md-bullet";
    bullet.textContent = "•";
    return bullet;
  }
}

class RuleWidget extends WidgetType {
  override eq() {
    return true;
  }
  override toDOM() {
    const rule = document.createElement("span");
    rule.className = "cm-md-rule";
    return rule;
  }
}

const hidden = Decoration.replace({});
const bullet = Decoration.replace({ widget: new BulletWidget() });
const rule = Decoration.replace({ widget: new RuleWidget() });
const mark = (className: string) => Decoration.mark({ class: className });
const line = (className: string) => Decoration.line({ class: className });

const INLINE_STYLE: Readonly<Record<string, string>> = {
  StrongEmphasis: "cm-md-strong",
  Emphasis: "cm-md-em",
  Strikethrough: "cm-md-strike",
  InlineCode: "cm-md-code",
  Link: "cm-md-link",
};

/** Lines touched by any selection range show their raw Markdown. */
function activeLines(state: EditorState): Set<number> {
  const lines = new Set<number>();
  for (const range of state.selection.ranges) {
    const first = state.doc.lineAt(range.from).number;
    const last = state.doc.lineAt(range.to).number;
    for (let number = first; number <= last; number += 1) lines.add(number);
  }
  return lines;
}

/** YAML frontmatter, if the document opens with one, as [first, last] line numbers. */
function frontmatter(state: EditorState): readonly [number, number] | null {
  if (state.doc.lines < 2 || state.doc.line(1).text.trimEnd() !== "---") {
    return null;
  }
  for (let number = 2; number <= Math.min(state.doc.lines, 200); number += 1) {
    if (state.doc.line(number).text.trimEnd() === "---") return [1, number];
  }
  return null;
}

function decorate(view: EditorView): DecorationSet {
  const { state } = view;
  const active = activeLines(state);
  const ranges: Range<Decoration>[] = [];
  const lineOf = (position: number) => state.doc.lineAt(position);
  const inactive = (position: number) => !active.has(lineOf(position).number);
  const eachLine = (from: number, to: number, className: string) => {
    for (
      let number = lineOf(from).number;
      number <= lineOf(to).number;
      number += 1
    ) {
      ranges.push(line(className).range(state.doc.line(number).from));
    }
  };

  const yaml = frontmatter(state);
  if (yaml !== null) {
    const [first, last] = yaml;
    for (let number = first; number <= last; number += 1) {
      const edge =
        number === first
          ? " cm-md-fm-first"
          : number === last
            ? " cm-md-fm-last"
            : "";
      ranges.push(line(`cm-md-fm${edge}`).range(state.doc.line(number).from));
    }
    // The fences read as noise unless the cursor is inside the frontmatter.
    const editing = [...active].some(
      (number) => number >= first && number <= last,
    );
    if (!editing) {
      for (const number of [first, last]) {
        const fence = state.doc.line(number);
        if (fence.to > fence.from)
          ranges.push(hidden.range(fence.from, fence.to));
      }
    }
  }

  // Inside frontmatter, `---` and `key: value` must never read as Markdown.
  const bodyStart = yaml === null ? 0 : state.doc.line(yaml[1]).to + 1;
  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from: Math.max(from, bodyStart),
      to,
      enter: (node) => {
        if (node.from < bodyStart) return;
        const heading = /^(?:ATX|Setext)Heading(\d)$/.exec(node.name);
        if (heading !== null) {
          eachLine(node.from, node.to, `cm-md-h cm-md-h${heading[1]}`);
          return;
        }
        const style = INLINE_STYLE[node.name];
        if (style !== undefined && node.to > node.from) {
          ranges.push(mark(style).range(node.from, node.to));
        }
        switch (node.name) {
          case "Blockquote":
            eachLine(node.from, node.to, "cm-md-quote");
            return;
          case "FencedCode":
            eachLine(node.from, node.to, "cm-md-codeblock");
            return;
          case "HeaderMark": {
            if (!inactive(node.from)) return;
            // Hide "## " including the space that follows the marks.
            const after = state.doc.sliceString(node.to, node.to + 1);
            ranges.push(
              hidden.range(node.from, node.to + (after === " " ? 1 : 0)),
            );
            return;
          }
          case "EmphasisMark":
          case "StrikethroughMark":
          case "QuoteMark":
          case "LinkMark":
          case "URL":
            if (
              inactive(node.from) &&
              node.node.parent?.name !== "FencedCode"
            ) {
              // A bare autolink is its own URL; only hide URLs inside [text](url).
              if (node.name === "URL" && node.node.parent?.name !== "Link")
                return;
              ranges.push(hidden.range(node.from, node.to));
            }
            return;
          case "CodeMark":
            if (
              node.node.parent?.name === "InlineCode" &&
              inactive(node.from)
            ) {
              ranges.push(hidden.range(node.from, node.to));
            }
            return;
          case "ListMark": {
            const ordered = node.node.parent?.parent?.name === "OrderedList";
            if (ordered) {
              ranges.push(mark("cm-md-listnum").range(node.from, node.to));
            } else if (inactive(node.from)) {
              ranges.push(bullet.range(node.from, node.to));
            }
            return;
          }
          case "HorizontalRule":
            if (inactive(node.from))
              ranges.push(rule.range(node.from, node.to));
            return;
          default:
        }
      },
    });
  }
  return Decoration.set(ranges, true);
}

export const liveMarkdown = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = decorate(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = decorate(update.view);
      }
    }
  },
  { decorations: (plugin) => plugin.decorations },
);
