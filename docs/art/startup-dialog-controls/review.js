for (const compare of document.querySelectorAll('[data-compare]')) {
  const range = compare.closest('.card').querySelector('.compare-range');
  const update = value => {
    const position = Math.max(0, Math.min(100, Number(value)));
    compare.style.setProperty('--split', `${position}%`);
    range.value = String(position);
  };
  range.addEventListener('input', () => update(range.value));
  const move = event => {
    const bounds = compare.getBoundingClientRect();
    update((event.clientX - bounds.left) * 100 / bounds.width);
  };
  compare.addEventListener('pointerdown', event => {
    compare.setPointerCapture(event.pointerId);
    move(event);
  });
  compare.addEventListener('pointermove', event => {
    if (compare.hasPointerCapture(event.pointerId)) move(event);
  });
}

const atlasPaths = {
  classic: '../../../planet/Graphics.c4g/StartupBookScroll.png',
  hd: '../../../crates/clonk-app/assets/StartupBookScrollHD.png',
  wipf: '../../../crates/clonk-app/assets/StartupWipfHD.png'
};
const loadImage = source => new Promise((resolve, reject) => {
  const image = new Image();
  image.onload = () => resolve(image);
  image.onerror = () => reject(new Error(`Could not load ${source}`));
  image.src = source;
});

Promise.all(Object.values(atlasPaths).map(loadImage)).then(([classic, hd, wipf]) => {
  const canvases = [document.getElementById('classic-scroll'), document.getElementById('hd-scroll')];
  const range = document.getElementById('wipf-position');
  const logicalHeight = 240;
  const scale = 3;
  let position = 0;
  let pressed = 0;

  const paint = (canvas, atlas, highResolution) => {
    const ctx = canvas.getContext('2d');
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.imageSmoothingEnabled = highResolution;
    const sourceScale = highResolution ? 8 : 1;
    const facet = (sourceX, sourceY, destinationY, height) => {
      ctx.drawImage(atlas, sourceX * sourceScale, sourceY * sourceScale,
        16 * sourceScale, height * sourceScale,
        0, destinationY * scale, 16 * scale, height * scale);
    };
    facet(pressed === -1 ? 16 : 0, 0, 0, 16);
    for (let y = 16; y < logicalHeight - 5; y += 16) {
      facet(0, 16, y, Math.min(16, logicalHeight - 5 - y));
    }
    facet(pressed === 1 ? 16 : 0, 32, logicalHeight - 16, 16);
    if (highResolution) {
      ctx.drawImage(wipf, 0, (16 + position) * scale, 16 * scale, 16 * scale);
    } else {
      ctx.drawImage(atlas, 16, 16, 16, 16, 0, (16 + position) * scale, 16 * scale, 16 * scale);
    }
  };
  const render = () => {
    paint(canvases[0], classic, false);
    paint(canvases[1], hd, true);
    range.value = String(position);
  };
  const setPosition = value => {
    position = Math.max(0, Math.min(logicalHeight - 48, Math.round(value)));
    render();
  };
  range.addEventListener('input', () => setPosition(range.value));
  for (const canvas of canvases) {
    const logicalY = event => (event.clientY - canvas.getBoundingClientRect().top) * logicalHeight / canvas.getBoundingClientRect().height;
    canvas.addEventListener('pointerdown', event => {
      canvas.setPointerCapture(event.pointerId);
      const y = logicalY(event);
      if (y < 16) { pressed = -1; setPosition(position - 12); }
      else if (y >= logicalHeight - 16) { pressed = 1; setPosition(position + 12); }
      else setPosition(y - 24);
    });
    canvas.addEventListener('pointermove', event => {
      if (canvas.hasPointerCapture(event.pointerId) && pressed === 0) setPosition(logicalY(event) - 24);
    });
    const release = () => { pressed = 0; render(); };
    canvas.addEventListener('pointerup', release);
    canvas.addEventListener('pointercancel', release);
  }
  render();
}).catch(error => {
  document.getElementById('asset-status').textContent = error.message;
});
