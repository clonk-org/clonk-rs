const art = {
  normal: { before: "../../../planet/Graphics.c4g/StartupBigButton.png", previous: "button-normal-hd-v4.png", after: "../../../planet/Graphics.c4g/StartupBigButtonHD.png" },
  pressed: { before: "../../../planet/Graphics.c4g/StartupBigButtonDown.png", previous: "button-pressed-hd-v4.png", after: "../../../planet/Graphics.c4g/StartupBigButtonDownHD.png" },
};

const images = {};
const selectors = {
  label: document.getElementById("button-label"),
  width: document.getElementById("button-width"),
  widthValue: document.getElementById("width-value"),
  normal: document.getElementById("normal-state"),
  pressed: document.getElementById("pressed-state"),
  compare: document.getElementById("compare-range"),
  wipeStage: document.getElementById("wipe-stage"),
  wipeAfter: document.getElementById("wipe-after"),
  divider: document.getElementById("wipe-divider"),
  grainCompare: document.getElementById("grain-compare-range"),
  grainStage: document.getElementById("grain-wipe-stage"),
  grainAfter: document.getElementById("grain-wipe-after"),
  grainDivider: document.getElementById("grain-wipe-divider"),
};

let state = new URLSearchParams(window.location.search).get("state") === "pressed" ? "pressed" : "normal";

function loadImage(source) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(`Could not load ${source}`));
    image.src = source;
  });
}

function drawThreeSlice(canvas, image, label, width, displayWidth) {
  const pixelRatio = window.devicePixelRatio || 1;
  const displayHeight = displayWidth * 40 / width;
  canvas.width = Math.round(displayWidth * pixelRatio);
  canvas.height = Math.round(displayHeight * pixelRatio);
  canvas.style.width = `${displayWidth}px`;
  canvas.style.height = `${displayHeight}px`;
  const context = canvas.getContext("2d");
  context.setTransform(canvas.width / width, 0, 0, canvas.height / 40, 0, 0);
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = "high";

  const sourceScale = image.height / 40;
  const border = 40;
  const sourceBorder = sourceScale > 1 ? 200 : border;
  const middle = image.width - 2 * sourceBorder;
  context.drawImage(image, 0, 0, sourceBorder, image.height, 0, 0, border, 40);
  if (sourceScale > 1) {
    context.drawImage(image, sourceBorder, 0, middle, image.height, border, 0, width - 2 * border, 40);
  } else {
    for (let x = border; x < width - Math.floor(border / 3); x += middle) {
      const sliceWidth = Math.min(middle, width - Math.floor(border / 3) - x);
      context.drawImage(image, sourceBorder, 0, sliceWidth, image.height, x, 0, sliceWidth, 40);
    }
  }
  context.drawImage(image, image.width - sourceBorder, 0, sourceBorder, image.height, width - border, 0, border, 40);

  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillStyle = "#fff000";
  context.shadowColor = "#533000";
  context.shadowOffsetY = 1;
  context.shadowBlur = 0.7;
  context.font = '20px Endeavour, Georgia, serif';
  const textOffset = state === "pressed" ? 1 : 0;
  context.fillText(label, width / 2 + textOffset, 19 + textOffset);
}

function draw() {
  if (!images.normal?.before) return;
  const width = Number(selectors.width.value);
  const label = selectors.label.value;
  const stage = document.querySelector(".comparison-row .button-stage");
  const displayWidth = Math.min(stage.clientWidth - 20, width * 2.7);
  selectors.widthValue.textContent = `${width} px`;

  drawThreeSlice(document.getElementById("before-canvas"), images[state].before, label, width, displayWidth);
  drawThreeSlice(document.getElementById("after-canvas"), images[state].after, label, width, displayWidth);
  drawThreeSlice(document.getElementById("wipe-before"), images[state].before, label, width, displayWidth);
  drawThreeSlice(selectors.wipeAfter, images[state].after, label, width, displayWidth);
  drawThreeSlice(document.getElementById("grain-before-canvas"), images[state].previous, label, width, displayWidth);
  drawThreeSlice(document.getElementById("grain-after-canvas"), images[state].after, label, width, displayWidth);
  drawThreeSlice(document.getElementById("grain-wipe-before"), images[state].previous, label, width, displayWidth);
  drawThreeSlice(selectors.grainAfter, images[state].after, label, width, displayWidth);

  for (const stage of [selectors.wipeStage, selectors.grainStage]) {
    stage.style.height = `${displayWidth * 40 / width + 12}px`;
    stage.style.width = `${Math.min(stage.parentElement.clientWidth - 34, displayWidth + 18)}px`;
  }
  updateDivider();
}

function updateDivider() {
  for (const [range, after, divider] of [
    [selectors.compare, selectors.wipeAfter, selectors.divider],
    [selectors.grainCompare, selectors.grainAfter, selectors.grainDivider],
  ]) {
    const percent = Number(range.value);
    after.style.clipPath = `inset(0 0 0 ${percent}%)`;
    divider.style.left = `${percent}%`;
  }
}

function setState(next) {
  state = next;
  selectors.normal.setAttribute("aria-pressed", String(next === "normal"));
  selectors.pressed.setAttribute("aria-pressed", String(next === "pressed"));
  draw();
}

selectors.normal.addEventListener("click", () => setState("normal"));
selectors.pressed.addEventListener("click", () => setState("pressed"));
selectors.label.addEventListener("change", draw);
selectors.width.addEventListener("input", draw);
selectors.compare.addEventListener("input", updateDivider);
selectors.grainCompare.addEventListener("input", updateDivider);
document.querySelectorAll("canvas.rendered").forEach((canvas) => canvas.addEventListener("click", () => setState(state === "normal" ? "pressed" : "normal")));

function dragDivider(event, stage, range) {
  const bounds = stage.getBoundingClientRect();
  range.value = String(Math.max(0, Math.min(100, Math.round((event.clientX - bounds.left) / bounds.width * 100))));
  updateDivider();
}
for (const [stage, range] of [[selectors.wipeStage, selectors.compare], [selectors.grainStage, selectors.grainCompare]]) {
  stage.addEventListener("pointerdown", (event) => {
    stage.setPointerCapture(event.pointerId);
    dragDivider(event, stage, range);
  });
  stage.addEventListener("pointermove", (event) => {
    if (stage.hasPointerCapture(event.pointerId)) dragDivider(event, stage, range);
  });
}
window.addEventListener("resize", draw);

Promise.all([...Object.entries(art).flatMap(([name, sources]) => Object.entries(sources).map(async ([variant, source]) => {
  images[name] ||= {};
  images[name][variant] = await loadImage(source);
})), document.fonts.load("20px Endeavour")]).then(() => {
  setState(state);
}).catch((error) => {
  document.querySelector(".workbench").insertAdjacentText("beforeend", error.message);
});
