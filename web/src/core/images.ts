// Image files, by name, as object URLs: fetched from the server once, with the device token.
import { token } from "./token";

const images = new Map<string, Promise<string>>();

export function imageUrl(name: string): Promise<string> {
  let url = images.get(name);
  if (!url) {
    url = fetch(`/v1/blobs/${encodeURIComponent(name)}`, { headers: { Authorization: `Bearer ${token() ?? ""}` } }).then(async (response) => {
      if (!response.ok) throw new Error(`image ${name}: ${response.status}`);
      return URL.createObjectURL(await response.blob());
    });
    url.catch(() => images.delete(name));
    images.set(name, url);
  }
  return url;
}
