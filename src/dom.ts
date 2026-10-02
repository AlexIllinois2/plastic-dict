import { parseDocument } from "htmlparser2";
import { Element, Text, Document, type AnyNode } from "domhandler";
import * as DomSerializer from "dom-serializer";

export function isElem(n: AnyNode): n is Element {
  return n.type === "tag" || n.type === "script" || n.type === "style";
}

export function tagOf(n: AnyNode): string {
  if (!isElem(n)) return "";
  const name = n.name;
  const i = name.indexOf(":");
  return i >= 0 ? name.slice(i + 1) : name;
}

export function* iterAll(n: AnyNode): Generator<Element> {
  if (isElem(n)) yield n;
  const kids = (n as Element).children;
  if (kids) for (const c of kids) yield* iterAll(c);
}

export function childElems(n: AnyNode): Element[] {
  const kids = (n as Element).children ?? [];
  return kids.filter(isElem) as Element[];
}

export function textContent(n: AnyNode | null | undefined): string {
  if (!n) return "";
  let out = "";
  const visit = (x: AnyNode): void => {
    if (x.type === "text") out += (x as Text).data;
    const kids = (x as Element).children;
    if (kids) for (const c of kids) visit(c);
  };
  visit(n);
  return out;
}

export function attrOf(el: Element, name: string): string | undefined {
  if (!el.attribs) return undefined;
  return el.attribs[name];
}

export function setAttr(el: Element, name: string, value: string): void {
  if (!el.attribs) el.attribs = {};
  el.attribs[name] = value;
}

export function delAttr(el: Element, name: string): void {
  if (el.attribs) delete el.attribs[name];
}

export function dropTree(el: Element): void {
  const p = el.parent as Element | Document | null;
  if (!p || !p.children) return;
  const i = p.children.indexOf(el);
  if (i >= 0) p.children.splice(i, 1);
}

export function dropTag(el: Element): void {
  const p = el.parent as Element | Document | null;
  if (!p || !p.children) return;
  const i = p.children.indexOf(el);
  if (i < 0) return;
  const kids = el.children ?? [];
  p.children.splice(i, 1, ...kids);
  for (const c of kids) c.parent = p;
  el.children = [];
}

export function parseHtml(html: string): Document | null {
  try {
    return parseDocument(html, { decodeEntities: true });
  } catch {
    return null;
  }
}

export function serializeDoc(doc: Document): string {
  const kids = doc.children ?? [];
  return kids
    .map((k) => DomSerializer.render(k as Element, { decodeEntities: true }))
    .join("");
}
