// The picture of an image row, fetched from the server with the device token.
import { useEffect, useState } from "preact/hooks";
import { imageUrl } from "../core/images";

export function RowImage({ name, text }: { name: string; text: string }) {
  const [src, setSrc] = useState<string>();
  const [missing, setMissing] = useState(false);
  useEffect(() => {
    let live = true;
    setMissing(false);
    imageUrl(name).then(
      (url) => live && setSrc(url),
      () => live && setMissing(true),
    );
    return () => {
      live = false;
    };
  }, [name]);
  return <img class="image" src={src} alt={missing ? `${text} (image not on the server)` : text} />;
}
