const { newContext } = require("./harness.js");
const { test, ok, eq } = require("./run.js");
function fixture() {
  const ctx = newContext(); ctx.load("00-state.js", "02-textures.js");
  const images = [];
  ctx.set("Image", function () {
    const image = { naturalWidth: 64, naturalHeight: 64, src: "", removeAttribute() { this.src = ""; } };
    images.push(image); return image;
  });
  const shape = (id, color = 0) => { ctx.set("shapeArgs", [id, color]); return ctx.run("lightShape(...shapeArgs)"); };
  return { ctx, images, shape };
}

test("temporary light failure retries after backoff and recovers the original mask", () => {
  const { ctx, images, shape } = fixture();
  eq(shape(1, 2), null); eq(images[0].src, "light/1.png?c=2"); images[0].onerror();
  eq(shape(1, 2), null); eq(images.length, 1);
  ctx.setNow(2000); shape(1, 2); eq(images.length, 2);
  images[1].onload(); eq(shape(1, 2), images[1]); eq(ctx.get("lightShapeLoads"), 0);
});

test("light load admission and deadlines release stalled requests and ignore late callbacks", () => {
  const { ctx, images, shape } = fixture();
  for (let i = 0; i < 20; i++) shape(i);
  eq(images.length, 8); eq(ctx.get("lightShapeLoads"), 8);
  const late = images[0].onload;
  ctx.advance(5000); eq(ctx.get("lightShapeLoads"), 0);
  ok(images.every(img => !img.src));
  late(); eq(ctx.get("lightShapeBytes"), 0);
  shape(10); eq(images.length, 9); images[8].onload(); eq(shape(10), images[8]);
});

test("cold light variants are evicted while recently drawn masks remain usable", () => {
  const { ctx, images, shape } = fixture();
  for (let color = 0; color < 256; color++) { shape(1, color); images.at(-1).onload(); }
  ctx.setNow(2000); eq(shape(1, 0), images[0]);
  shape(1, 256); images.at(-1).onload();
  eq(ctx.get("lightShapes").size, 256); ok(images[0].src); eq(images[1].src, "");
});

test("decoded light byte pressure releases cold masks and retains live ones", () => {
  const { ctx, images, shape } = fixture();
  for (let id = 0; id < 5; id++) {
    shape(id); images.at(-1).naturalWidth = 1024; images.at(-1).naturalHeight = 1024; images.at(-1).onload();
  }
  eq(ctx.get("lightShapeBytes"), 20 * 1024 * 1024);
  ctx.setNow(2000); shape(0);
  eq(ctx.get("lightShapeBytes"), 16 * 1024 * 1024);
  ok(images[0].src); eq(images[1].src, "");

});

test("invalid identifiers and oversized decoded lights do not remain cached", () => {
  const { ctx, images, shape } = fixture();
  for (const [id, color] of [[-1, 0], [100, 0], [1.5, 0], [1, -1], [1, 65536], [1, NaN]]) eq(shape(id, color), null);
  eq(images.length, 0);
  shape(1); images[0].naturalWidth = 100000; images[0].onload();
  eq(ctx.get("lightShapeBytes"), 0); eq(images[0].src, ""); eq(shape(1), null);
});


test("the regular graphics sweep releases excess light memory after lights leave the scene", () => {
  const { ctx, images, shape } = fixture();
  for (let id = 0; id < 5; id++) {
    shape(id); images.at(-1).naturalWidth = 1024; images.at(-1).naturalHeight = 1024; images.at(-1).onload();
  }
  eq(ctx.get("lightShapeBytes"), 20 * 1024 * 1024);
  ctx.setNow(2000); ctx.run("sweepTexCache()");
  eq(ctx.get("lightShapeBytes"), 16 * 1024 * 1024);
});
