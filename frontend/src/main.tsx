import React, { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  Annotorious,
  ImageAnnotator,
  useAnnotator,
  type AnnotoriousImageAnnotator,
  type ImageAnnotation,
} from "@annotorious/react";
import "@annotorious/react/annotorious-react.css";
import { commentOf, copyText, type ImageMeta } from "./copy";
import "./style.css";

type Envelope = { schema: "annotorious-v3"; annotation: ImageAnnotation };
async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch("/api" + path, init);
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(body?.error?.message || `HTTP ${response.status}`);
  }
  return response.status === 204 ? (undefined as T) : response.json();
}
function App() {
  const [images, setImages] = useState<ImageMeta[]>([]),
    [image, setImage] = useState<ImageMeta | null>(null),
    [error, setError] = useState("");
  const flush = useRef<() => Promise<void>>(async () => {});
  const current = useRef<string | null>(null),
    pending = useRef<ImageMeta | null>(null),
    switching = useRef(false);
  const switchTo = useCallback(async (next: ImageMeta) => {
    pending.current = next;
    if (switching.current) return;
    switching.current = true;
    try {
      while (pending.current) {
        const target: ImageMeta = pending.current;
        if (current.current !== target.id) {
          await flush.current();
          current.current = target.id;
          setImage(target);
        }
        if (pending.current === target) pending.current = null;
      }
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      switching.current = false;
    }
  }, []);
  const refresh = useCallback(async () => {
    try {
      const [list, now] = await Promise.all([
        api<{ images: ImageMeta[] }>("/images"),
        api<{ image: ImageMeta | null }>("/current"),
      ]);
      setImages(list.images);
      if (now.image) await switchTo(now.image);
    } catch (e) {
      setError(String(e));
    }
  }, [switchTo]);
  useEffect(() => {
    const events = new EventSource("/api/events");
    events.onopen = () => void refresh();
    events.addEventListener("image", () => void refresh());
    events.onerror = () => setError("Connection interrupted. Reconnecting…");
    return () => events.close();
  }, [refresh]);
  return (
    <>
      <header>
        <h1>
          Markframe <span>LOCAL REVIEW</span>
        </h1>
        <select
          aria-label="Image history"
          value={image?.id || ""}
          onChange={(e) => {
            const next = images.find((i) => i.id === e.target.value);
            if (next) void switchTo(next);
          }}
        >
          <option value="" disabled>
            No images
          </option>
          {images.map((i) => (
            <option key={i.id} value={i.id}>
              {i.name || i.filename} · {i.created_at}
            </option>
          ))}
        </select>
      </header>
      {error && (
        <div role="alert" className="error">
          {error}{" "}
          <button
            onClick={() =>
              pending.current ? void switchTo(pending.current) : void refresh()
            }
          >
            Retry
          </button>
        </div>
      )}
      {image ? (
        <Annotorious key={image.id}>
          <Editor
            image={image}
            register={(fn) => {
              flush.current = fn;
            }}
          />
        </Annotorious>
      ) : (
        <main className="empty">
          <div>
            <h2>A place to look closer.</h2>
            <p>Post an image, mark a region, and copy your feedback.</p>
            <code>markframe post output.png</code>
          </div>
        </main>
      )}
    </>
  );
}
function Editor({
  image,
  register,
}: {
  image: ImageMeta;
  register: (fn: () => Promise<void>) => void;
}) {
  const anno = useAnnotator<AnnotoriousImageAnnotator>();
  const [selected, setSelected] = useState<ImageAnnotation | null>(null),
    [comment, setComment] = useState(""),
    [status, setStatus] = useState("Loading annotations…"),
    [error, setError] = useState(""),
    [ready, setReady] = useState(false),
    [pan, setPan] = useState(false),
    [scale, setScale] = useState(1);
  const viewport = useRef<HTMLDivElement>(null),
    selectedId = useRef<string | null>(null),
    pending = useRef(new Map<string, ImageAnnotation>()),
    persisted = useRef(new Set<string>()),
    saving = useRef<Promise<void> | null>(null),
    timer = useRef<ReturnType<typeof setTimeout> | null>(null),
    gesture = useRef(false);
  const loaded = useRef(false),
    mounted = useRef(true);
  const endpoint = `/images/${image.id}/annotations`;
  const flush = useCallback(async () => {
    while (saving.current) {
      await saving.current;
    }
    if (!loaded.current)
      throw new Error(
        "Annotations are still loading. Retry after loading completes.",
      );
    const work = async () => {
      while (pending.current.size) {
        const [id, annotation] = pending.current.entries().next().value!;
        setStatus("Saving…");
        await api(
          endpoint +
            (persisted.current.has(id) ? "/" + encodeURIComponent(id) : ""),
          {
            method: persisted.current.has(id) ? "PUT" : "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ schema: "annotorious-v3", annotation }),
          },
        );
        persisted.current.add(id);
        if (pending.current.get(id) === annotation) pending.current.delete(id);
      }
      if (mounted.current) {
        setStatus("Saved");
        setError("");
      }
    };
    const promise = work();
    saving.current = promise;
    try {
      await promise;
    } catch (e) {
      if (mounted.current) {
        setError(String(e));
        setStatus("Unsaved changes");
      }
      throw e;
    } finally {
      if (saving.current === promise) saving.current = null;
    }
  }, [endpoint]);
  const queue = useCallback(
    (a: ImageAnnotation) => {
      pending.current.set(a.id, a);
      setStatus("Unsaved changes");
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => void flush().catch(() => {}), 400);
    },
    [flush],
  );
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);
  useEffect(() => {
    register(async () => {
      if (gesture.current)
        await new Promise<void>((resolve) => {
          const done = () => {
            window.removeEventListener("pointerup", done, true);
            window.removeEventListener("pointercancel", done, true);
            setTimeout(resolve, 50);
          };
          window.addEventListener("pointerup", done, true);
          window.addEventListener("pointercancel", done, true);
        });
      if (anno && loaded.current) {
        for (const a of anno.getAnnotations()) {
          pending.current.set(a.id, a);
        }
      }
      await flush();
    });
  });
  useEffect(() => {
    const warn = (e: BeforeUnloadEvent) => {
      if (pending.current.size) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, []);
  useEffect(() => {
    if (!anno) return;
    let cancelled = false;
    const load = async () => {
      try {
        const data = await api<{ annotations: Envelope[] }>(endpoint);
        if (cancelled) return;
        anno.setAnnotations(data.annotations.map((e) => e.annotation));
        persisted.current = new Set(
          data.annotations.map((e) => e.annotation.id),
        );
        loaded.current = true;
        setReady(true);
        setStatus("Saved");
        setError("");
      } catch (e) {
        setError(String(e));
        setStatus("Could not load annotations");
      }
    };
    void load();
    const select = (items: ImageAnnotation[]) => {
      const a = items[0] || null;
      selectedId.current = a?.id || null;
      setSelected(a);
      setComment(a ? commentOf(a) : "");
    };
    const create = (a: ImageAnnotation) => {
      queue(a);
      anno.setSelected(a.id);
      select([a]);
    };
    const update = (a: ImageAnnotation) => {
      queue(a);
      if (selectedId.current === a.id) setSelected(a);
    };
    anno.on("createAnnotation", create);
    anno.on("updateAnnotation", update);
    anno.on("selectionChanged", select);
    return () => {
      cancelled = true;
      anno.off("createAnnotation", create);
      anno.off("updateAnnotation", update);
      anno.off("selectionChanged", select);
    };
  }, [anno, endpoint, queue]);
  const fit = useCallback(() => {
    const el = viewport.current;
    if (el)
      setScale(
        Math.min(
          (el.clientWidth - 48) / image.width,
          (el.clientHeight - 48) / image.height,
          1,
        ),
      );
  }, [image]);
  useEffect(() => {
    fit();
  }, [fit]);
  useEffect(() => {
    const up = () => {
      gesture.current = false;
    };
    window.addEventListener("pointerup", up, true);
    window.addEventListener("pointercancel", up, true);
    return () => {
      window.removeEventListener("pointerup", up, true);
      window.removeEventListener("pointercancel", up, true);
    };
  }, []);
  function edit(value: string) {
    if (!anno || !selected) return;
    setComment(value);
    const latest = anno.getAnnotationById(selected.id) as ImageAnnotation;
    const old = latest.bodies.find((b) => b.purpose === "commenting");
    const annotation = {
      ...latest,
      bodies: [
        ...latest.bodies.filter((b) => b.purpose !== "commenting"),
        {
          ...old,
          id: old?.id || crypto.randomUUID(),
          annotation: latest.id,
          purpose: "commenting",
          value,
        },
      ],
    };
    anno.updateAnnotation(annotation);
    setSelected(annotation);
    queue(annotation);
  }
  async function copy() {
    try {
      const latest = anno.getAnnotationById(
        selectedId.current!,
      ) as ImageAnnotation;
      pending.current.set(latest.id, latest);
      await flush();
      const a = anno.getAnnotationById(selectedId.current!) as ImageAnnotation;
      await navigator.clipboard.writeText(copyText(image, a));
      setStatus("Copied");
    } catch (e) {
      setError(String(e));
    }
  }
  async function remove() {
    if (!selected) return;
    try {
      await flush();
      await api(endpoint + "/" + encodeURIComponent(selected.id), {
        method: "DELETE",
      });
      anno.removeAnnotation(selected.id);
      persisted.current.delete(selected.id);
      setSelected(null);
      selectedId.current = null;
      setComment("");
      setStatus("Deleted");
    } catch (e) {
      setError(String(e));
    }
  }
  const drag = useRef<{
    x: number;
    y: number;
    left: number;
    top: number;
  } | null>(null);
  return (
    <main className="workspace">
      <div className="toolbar">
        <div>
          <button aria-pressed={!pan} onClick={() => setPan(false)}>
            Rectangle
          </button>
          <button aria-pressed={pan} onClick={() => setPan(true)}>
            Pan
          </button>
        </div>
        <div>
          <button
            aria-label="Zoom out"
            onClick={() => setScale((s) => Math.max(0.05, s / 1.25))}
          >
            −
          </button>
          <output>{Math.round(scale * 100)}%</output>
          <button
            aria-label="Zoom in"
            onClick={() => setScale((s) => Math.min(8, s * 1.25))}
          >
            +
          </button>
          <button onClick={fit}>Fit</button>
          <button onClick={() => setScale(1)}>Reset</button>
        </div>
        <span>
          {image.width} × {image.height}
        </span>
      </div>
      <div
        className={"viewport" + (pan ? " pan" : "")}
        ref={viewport}
        onPointerDownCapture={(e) => {
          gesture.current = true;
          if (pan) {
            e.stopPropagation();
            const el = viewport.current!;
            drag.current = {
              x: e.clientX,
              y: e.clientY,
              left: el.scrollLeft,
              top: el.scrollTop,
            };
            el.setPointerCapture(e.pointerId);
            e.preventDefault();
          }
        }}
        onPointerMove={(e) => {
          if (drag.current) {
            const d = drag.current;
            viewport.current!.scrollLeft = d.left + d.x - e.clientX;
            viewport.current!.scrollTop = d.top + d.y - e.clientY;
          }
        }}
        onPointerUp={() => {
          drag.current = null;
          gesture.current = false;
        }}
        onPointerCancel={() => {
          drag.current = null;
          gesture.current = false;
        }}
      >
        <div
          className="image-space"
          style={{
            width: Math.max(
              image.width * scale + 48,
              viewport.current?.clientWidth || 0,
            ),
            minHeight: "100%",
          }}
        >
          <div
            className="image-frame"
            style={{
              width: image.width * scale,
              pointerEvents: pan || !ready ? "none" : "auto",
            }}
          >
            <ImageAnnotator
              tool="rectangle"
              autoSave
              drawingEnabled={!pan && ready}
            >
              <img
                draggable={false}
                src={`/api/images/${image.id}/content`}
                alt={image.filename}
                style={{
                  width: image.width * scale,
                  height: image.height * scale,
                }}
                onLoad={fit}
              />
            </ImageAnnotator>
          </div>
        </div>
      </div>
      <section className="inspector">
        <div className="inspector-title">
          <label htmlFor="comment">
            {selected ? "Selected annotation" : "Draw a rectangle to annotate"}
          </label>
          <span role="status" aria-label="Save status">
            {status}
          </span>
        </div>
        <textarea
          id="comment"
          disabled={!selected}
          value={comment}
          placeholder="Your comment"
          onChange={(e) => edit(e.target.value)}
        />
        <div className="actions">
          <span>Comments are copied exactly as written.</span>
          <button disabled={!selected} onClick={() => void remove()}>
            Delete
          </button>
          <button
            className="primary"
            disabled={!selected}
            onClick={() => void copy()}
          >
            Copy
          </button>
        </div>
        {error && (
          <div role="alert" className="error">
            {error}{" "}
            <button
              onClick={() => {
                if (loaded.current) void flush().catch(() => {});
                else window.location.reload();
              }}
            >
              Retry
            </button>
          </div>
        )}
      </section>
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
