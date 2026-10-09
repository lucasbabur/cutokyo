import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { EditorSelection, EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { afterEach, describe, expect, it } from "vitest";

import { liveMarkdown } from "./markdownLive.js";

const views: EditorView[] = [];
afterEach(() => {
  for (const view of views.splice(0)) view.destroy();
});

function open(doc: string, cursor: number) {
  const view = new EditorView({
    parent: document.body,
    state: EditorState.create({
      doc,
      selection: EditorSelection.cursor(cursor),
      extensions: [
        yamlFrontmatter({ content: markdown({ base: markdownLanguage }) }),
        liveMarkdown,
      ],
    }),
  });
  views.push(view);
  return view;
}

const lines = (view: EditorView) =>
  [...view.contentDOM.querySelectorAll(".cm-line")] as HTMLElement[];

describe("live Markdown rendering", () => {
  const doc = "# Title\n\nSome **bold** and ~~old~~ text\n\n- item";

  it("renders marks away on lines that are not being edited", () => {
    const view = open(doc, doc.length);
    const [heading, , body, , item] = lines(view);
    expect(heading?.classList.contains("cm-md-h1")).toBe(true);
    expect(heading?.textContent).toBe("Title");
    expect(body?.textContent).toBe("Some bold and old text");
    expect(body?.querySelector(".cm-md-strong")?.textContent).toBe("bold");
    expect(body?.querySelector(".cm-md-strike")?.textContent).toBe("old");
    // The cursor's own line keeps its raw marks.
    expect(item?.textContent).toBe("- item");
    // Decorations never change the document.
    expect(view.state.doc.toString()).toBe(doc);
  });

  it("shows the raw Markdown of the line being edited", () => {
    const view = open(doc, 3);
    const [heading, , body] = lines(view);
    expect(heading?.textContent).toBe("# Title");
    expect(body?.textContent).toBe("Some bold and old text");
  });

  it("hides frontmatter fences until the cursor enters the frontmatter", () => {
    // Without a YAML-aware parser `---` would be a rule; it must still read as a fence.
    const front = "---\nname: x\n---\n\nBody";
    const outside = open(front, front.length);
    expect(lines(outside)[0]?.textContent).toBe("");
    expect(lines(outside)[0]?.classList.contains("cm-md-fm-first")).toBe(true);
    const inside = open(front, 5);
    expect(lines(inside)[0]?.textContent).toBe("---");
  });

  it("never styles frontmatter as Markdown, even when the parser reads a rule", () => {
    const front = "---\nname: x\n---\n\nBody";
    const view = new EditorView({
      parent: document.body,
      state: EditorState.create({
        doc: front,
        selection: EditorSelection.cursor(5),
        extensions: [markdown({ base: markdownLanguage }), liveMarkdown],
      }),
    });
    views.push(view);
    expect(lines(view)[0]?.textContent).toBe("---");
    expect(view.contentDOM.querySelector(".cm-md-rule")).toBeNull();
  });
});
