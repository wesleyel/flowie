// Asks for the device token (from `flowie-server token add`), then starts over with it.
import { setToken } from "../core/token";

export function Setup() {
  const onSubmit = (event: SubmitEvent) => {
    event.preventDefault();
    const input = (event.currentTarget as HTMLFormElement).elements.namedItem("token") as HTMLInputElement;
    setToken(input.value.trim());
    location.reload();
  };
  return (
    <form class="setup" onSubmit={onSubmit}>
      <label>
        Device token
        <br />
        <input name="token" autocomplete="off" placeholder="from flowie-server token add" />
      </label>
      <button>Connect</button>
    </form>
  );
}
