import { describe, it, expect } from "vitest";
import type { ImageAnnotation } from "@annotorious/react";
import { copyText, type ImageMeta } from "./copy";
const image = { id: "image-1", width: 1280, height: 720 } as ImageMeta;
describe("language-neutral clipboard", () => {
  for (const comment of [
    "この円を少し大きく",
    "make these circles farther apart",
    "",
    "  before\nafter \n",
    "😀\t日本語",
  ])
    it(JSON.stringify(comment), () => {
      const annotation = {
        id: "a",
        bodies: comment
          ? [
              {
                id: "b",
                annotation: "a",
                purpose: "commenting",
                value: comment,
              },
            ]
          : [],
        target: {
          annotation: "a",
          selector: {
            type: "RECTANGLE",
            geometry: {
              x: 10,
              y: 20,
              w: 30,
              h: 40,
              bounds: { minX: 10, minY: 20, maxX: 40, maxY: 60 },
            },
          },
        },
      } as ImageAnnotation;
      const copied = copyText(image, annotation);
      expect(
        copied.startsWith(comment ? comment + "\n\n```json\n" : "```json\n"),
      ).toBe(true);
      const parsed = JSON.parse(
        copied.slice(copied.indexOf("```json\n") + 8, -4),
      );
      expect(parsed).toEqual({
        image_id: "image-1",
        image_width: 1280,
        image_height: 720,
        annotation,
      });
    });
});
