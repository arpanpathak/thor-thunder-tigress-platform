// closeSvg makes a drawing that is still streaming into an SVG an image can show.
import assert from "node:assert/strict";
import { test } from "node:test";

import { closeSvg } from "../rich.js";

test("nothing is drawn before the opening tag is whole", () => {
  assert.equal(closeSvg(""), "");
  assert.equal(closeSvg("<svg viewBox=\"0 0 4"), "");
});

test("open elements are closed innermost first", () => {
  const partial = "<svg viewBox=\"0 0 4 4\"><g fill=\"red\"><rect width=\"4\"/><circle r";

  assert.equal(closeSvg(partial), "<svg viewBox=\"0 0 4 4\"><g fill=\"red\"><rect width=\"4\"/></g></svg>");
});

test("closed elements, comments and the XML header are left alone", () => {
  const partial = "<?xml version=\"1.0\"?>\n<svg><!-- sky --><text x=\"1\">Hi</text><path d=\"M0 0\"></path>";

  assert.equal(closeSvg(partial), "<svg><!-- sky --><text x=\"1\">Hi</text><path d=\"M0 0\"></path></svg>");
});

test("text cut mid-element keeps what is complete", () => {
  assert.equal(closeSvg("<svg><text>Hel"), "<svg><text></text></svg>");
});

test("a whole drawing comes back unchanged", () => {
  const whole = "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"4\" height=\"4\"/></svg>";

  assert.equal(closeSvg(whole), whole);
});
