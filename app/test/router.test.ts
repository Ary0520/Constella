import { linkIntent, matchRoute } from "../src/router";

describe("routes", () => {
  it("maps paths to screens", () => {
    expect(matchRoute("/")).toEqual({ name: "home", params: {} });
    expect(matchRoute("/new")).toEqual({ name: "new", params: {} });
    expect(matchRoute("/assets/")).toEqual({ name: "assets", params: {} });
    expect(matchRoute("/passport")).toEqual({ name: "passport", params: {} });
    expect(matchRoute("/account")).toEqual({ name: "account", params: {} });
    const v = "CDAJDSXBYMHJMGL35WXL73MDRLPLFOSEM4MZ4VZRWNY6F7XMFGAP4QQS";
    expect(matchRoute(`/vault/${v}`)).toEqual({ name: "vault", params: { id: v } });
  });
  it("refuses what is not a route or not a contract address", () => {
    expect(matchRoute("/vault/not-an-address").name).toBe("notfound");
    expect(matchRoute("/vault/GAD5SR3CXQLQUU64C6MN7VKRXRY5WCWYB3HT5JQI6NHV6BYX3MI5QRHF").name).toBe("notfound");
    expect(matchRoute("/admin").name).toBe("notfound");
  });
  it("reads the invitation links (device pairing, guardian, recovery approval)", () => {
    expect(linkIntent("?pair=ABCD2345")).toEqual({ kind: "pair", code: "ABCD2345" });
    expect(linkIntent("?guard=ABCD2345")).toEqual({ kind: "guard", code: "ABCD2345" });
    expect(linkIntent("?recover=ABCD2345")).toEqual({ kind: "recover", code: "ABCD2345" });
    expect(linkIntent("?pair=<script>")).toBeNull();
    expect(linkIntent("")).toBeNull();
  });
});
