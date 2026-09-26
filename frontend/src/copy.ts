import type { ImageAnnotation } from "@annotorious/react";
export type ImageMeta = {
  id: string;
  filename: string;
  name: string | null;
  width: number;
  height: number;
  mime_type: string;
  created_at: string;
};
export function commentOf(annotation: ImageAnnotation): string {
  return annotation.bodies
    .filter((b) => b.purpose === "commenting" && typeof b.value === "string")
    .map((b) => b.value)
    .join("\n");
}
export function copyText(
  image: ImageMeta,
  annotation: ImageAnnotation,
): string {
  const comment = commentOf(annotation);
  const json = JSON.stringify(
    {
      image_id: image.id,
      image_width: image.width,
      image_height: image.height,
      annotation,
    },
    null,
    2,
  );
  return (
    (comment.length ? comment + "\n\n" : "") + "```json\n" + json + "\n```"
  );
}
