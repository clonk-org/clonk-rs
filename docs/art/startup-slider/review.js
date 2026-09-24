// Draw the same 16-pixel facets and Wipf travel used by the Audio options UI.
const BAR_WIDTH = 320;
const DISPLAY_SCALE = 3;
const BAR_Y = 12;
const MAX_SCROLL = BAR_WIDTH - 48;
const positions = { music: 50, effects: 50 };
const canvases = [...document.querySelectorAll("canvas[data-channel]")];
const pressed = new WeakMap();
let sprites;

function loadImage(path) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(`Could not load ${path}`));
    image.src = path;
  });
}

function rotatedFacet(atlas, column, row) {
  const cell = atlas.naturalWidth / 3;
  const facet = document.createElement("canvas");
  facet.width = cell;
  facet.height = cell;
  const context = facet.getContext("2d");
  context.translate(0, cell);
  context.rotate(-Math.PI / 2);
  context.drawImage(atlas, column * cell, row * cell, cell, cell, 0, 0, cell, cell);
  return facet;
}

function facets(atlas) {
  return {
    cell: atlas.naturalWidth / 3,
    left: rotatedFacet(atlas, 0, 0),
    leftPressed: rotatedFacet(atlas, 1, 0),
    rail: rotatedFacet(atlas, 0, 1),
    right: rotatedFacet(atlas, 0, 2),
    rightPressed: rotatedFacet(atlas, 1, 2),
  };
}

function drawSlider(canvas) {
  const context = canvas.getContext("2d");
  const artwork = sprites[canvas.dataset.variant];
  const value = positions[canvas.dataset.channel];
  const activeArrow = pressed.get(canvas);
  const unit = 16 * DISPLAY_SCALE;
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = "high";

  context.drawImage(activeArrow === "left" ? artwork.leftPressed : artwork.left, 0, BAR_Y, unit, unit);
  for (let x = 16; x < BAR_WIDTH - 5; x += 16) {
    const width = Math.min(16, BAR_WIDTH - 5 - x);
    context.drawImage(
      artwork.rail,
      0, 0, width * artwork.cell / 16, artwork.cell,
      x * DISPLAY_SCALE, BAR_Y, width * DISPLAY_SCALE, unit,
    );
  }
  context.drawImage(
    activeArrow === "right" ? artwork.rightPressed : artwork.right,
    (BAR_WIDTH - 16) * DISPLAY_SCALE, BAR_Y, unit, unit,
  );
  const scrollPosition = Math.round(value * MAX_SCROLL / 100);
  context.drawImage(sprites.wipf, (16 + scrollPosition) * DISPLAY_SCALE, BAR_Y, unit, unit);
  canvas.setAttribute("aria-valuenow", String(value));
  canvas.setAttribute("aria-valuetext", `${value}%`);
}

function render() {
  if (sprites) canvases.forEach(drawSlider);
}

function setPosition(channel, value) {
  positions[channel] = Math.max(0, Math.min(100, Math.round(value)));
  document.getElementById(`${channel === "music" ? "music" : "effects"}-position`).value = positions[channel];
  document.getElementById(`${channel === "music" ? "music" : "effects"}-value`).textContent = `${positions[channel]}%`;
  render();
}

function pointerX(canvas, event) {
  const bounds = canvas.getBoundingClientRect();
  return (event.clientX - bounds.left) * BAR_WIDTH / bounds.width;
}

function setFromPointer(canvas, event) {
  setPosition(canvas.dataset.channel, (pointerX(canvas, event) - 24) * 100 / MAX_SCROLL);
}

function installControls() {
  for (const channel of ["music", "effects"]) {
    document.getElementById(`${channel}-position`).addEventListener("input", (event) => {
      setPosition(channel, Number(event.target.value));
    });
  }
  document.getElementById("reset").addEventListener("click", () => {
    setPosition("music", 50);
    setPosition("effects", 50);
  });

  for (const canvas of canvases) {
    canvas.addEventListener("pointerdown", (event) => {
      canvas.focus();
      canvas.setPointerCapture(event.pointerId);
      const x = pointerX(canvas, event);
      if (x < 16) {
        pressed.set(canvas, "left");
        setPosition(canvas.dataset.channel, positions[canvas.dataset.channel] - 5);
      } else if (x >= BAR_WIDTH - 16) {
        pressed.set(canvas, "right");
        setPosition(canvas.dataset.channel, positions[canvas.dataset.channel] + 5);
      } else {
        canvas.dataset.dragging = "true";
        setFromPointer(canvas, event);
      }
    });
    canvas.addEventListener("pointermove", (event) => {
      if (canvas.dataset.dragging === "true") setFromPointer(canvas, event);
    });
    for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
      canvas.addEventListener(name, () => {
        canvas.dataset.dragging = "false";
        pressed.delete(canvas);
        render();
      });
    }
    canvas.addEventListener("keydown", (event) => {
      const value = positions[canvas.dataset.channel];
      const next = {
        ArrowLeft: value - 1,
        ArrowDown: value - 1,
        ArrowRight: value + 1,
        ArrowUp: value + 1,
        PageDown: value - 10,
        PageUp: value + 10,
        Home: 0,
        End: 100,
      }[event.key];
      if (next === undefined) return;
      event.preventDefault();
      setPosition(canvas.dataset.channel, next);
    });
  }
}

async function start() {
  const [before, after, wipf] = await Promise.all([
    loadImage("../../../planet/Graphics.c4g/StartupBookScroll.png"),
    loadImage("../../../crates/clonk-app/assets/StartupBookScrollHD.png"),
    loadImage("../../../crates/clonk-app/assets/StartupWipfHD.png"),
  ]);
  sprites = { before: facets(before), after: facets(after), wipf };
  installControls();
  render();
}

start().catch((error) => {
  const message = document.getElementById("load-error");
  message.textContent = `Preview failed to load: ${error.message}`;
  message.hidden = false;
});
