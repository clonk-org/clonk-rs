"""Build the eight-cell navigation-arrow review page and manifest."""

from __future__ import annotations

import hashlib
import json
from html import escape
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
ICONS = [
    ("ARROW-01", "Up", "red-arrow", "Arrow.png", 0, 64, 64),
    ("ARROW-02", "Down", "red-arrow", "Arrow.png", 1, 64, 64),
    ("ARROW-03", "Left", "red-arrow", "Arrow.png", 2, 64, 64),
    ("ARROW-04", "Right", "red-arrow", "Arrow.png", 3, 64, 64),
    ("WOOD-01", "Player selection left", "wood-arrow", "GUIBigArrows.png", 0, 19, 40),
    ("WOOD-02", "Player selection right", "wood-arrow", "GUIBigArrows.png", 1, 19, 40),
    ("WOOD-03", "Player selection left pressed", "wood-arrow", "GUIBigArrows.png", 2, 19, 40),
    ("WOOD-04", "Player selection right pressed", "wood-arrow", "GUIBigArrows.png", 3, 19, 40),
]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


icons = []
for identifier, label, stem, sheet, phase, width, height in ICONS:
    before = HERE / "before" / f"{stem}-{phase}.png"
    after = HERE / "after" / f"{stem}-{phase}.png"
    game_size = HERE / "game-size" / f"{stem}-{phase}.png"
    assert Image.open(before).size == (width, height)
    assert Image.open(after).size == (width * 8, height * 8)
    assert Image.open(game_size).size == (width, height)
    icons.append(
        {
            "id": identifier,
            "label": label,
            "source": f"planet/Graphics.c4g/{sheet}",
            "sourceCell": [phase * width, 0, width, height],
            "sourceSha256": digest(ROOT / "planet/Graphics.c4g" / sheet),
            "before": f"before/{stem}-{phase}.png",
            "after": f"after/{stem}-{phase}.png",
            "gameSize": f"game-size/{stem}-{phase}.png",
            "afterSize": [width * 8, height * 8],
            "afterSha256": digest(after),
            "review": "approved",
        }
    )

(HERE / "manifest.json").write_text(
    json.dumps(
        {
            "title": "Clonk navigation-arrow icon review — batch 6",
            "status": "All eight approved and installed as high-resolution cell replacements",
            "revision": "Color and shading consistency pass",
            "method": "OpenAI built-in imagegen with foreground extraction and original-cell placement",
            "icons": icons,
            "otherLowResolutionCandidates": [
                "Rank.png: twenty-four 16x16 rank badges",
                "Options.png: sixteen remaining 35x35 cells",
                "Control.png: mixed-size keyboard and command facets",
                "GUIContext.png and GUISubmenu.png: tiny wooden menu controls",
                "Flag.png and Crew.png: owner-colored art requiring color-mask preservation",
            ],
        },
        indent=2,
    )
    + "\n"
)

cards = []
for icon in icons:
    identifier, label = escape(icon["id"]), escape(icon["label"])
    width, height = icon["sourceCell"][2:]
    phase = icon["sourceCell"][0] // width
    sheet = Path(icon["source"]).name
    after_version = icon["afterSha256"][:12]
    game_version = digest(HERE / icon["gameSize"])[:12]
    cards.append(
        f'''<article class="card" id="{identifier}" data-id="{identifier}">
<header><div><span class="id">{identifier}</span><h2>{label}</h2></div><span class="source">{sheet} · phase {phase}</span></header>
<div class="compare"><figure><figcaption>Original · {width}×{height}</figcaption><a href="{icon['before']}" target="_blank"><span class="frame"><img class="original" src="{icon['before']}" alt="Original {label}"></span></a></figure><figure><figcaption>High-resolution · {width * 8}×{height * 8}</figcaption><a href="{icon['after']}?v={after_version}" target="_blank"><span class="frame"><img src="{icon['after']}?v={after_version}" alt="Proposed {label}"></span></a></figure></div>
<p class="game-size">At game size · {width}×{height}: <img src="{icon['gameSize']}?v={game_version}" width="{width * 2}" height="{height * 2}" alt="Proposed {label} at game size"></p>
<div class="choices" role="group" aria-label="Review {label}"><button data-decision="approve" type="button">Approve</button><button data-decision="revise" type="button">Revise</button><button data-decision="keep" type="button">Keep original</button></div>
<textarea aria-label="Revision notes for {label}" placeholder="Optional detail to preserve or change"></textarea>
</article>'''
    )

html = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clonk navigation arrows — batch 6</title>
<style>
:root{--paper:#f5f2eb;--ink:#302d27;--muted:#716a60;--line:#ddd5c8;--accent:#75552b;--frame:#eae5da}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1420px;margin:auto;padding:34px 26px 100px}h1{font-size:38px;line-height:1.1;letter-spacing:-1px;margin:8px 0 12px}h2{font-size:19px;margin:1px 0}.eyebrow{font-size:12px;color:var(--accent);font-weight:700;text-transform:uppercase;letter-spacing:1.5px}.intro{max-width:960px;color:var(--muted);margin:0 0 18px}.intro strong{color:var(--ink)}.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;background:#fffdf8;border:1px solid var(--line);border-radius:10px;padding:11px 15px;margin:20px 0}.toolbar span:first-child{margin-right:auto;font-weight:650}button{font:inherit;cursor:pointer;padding:7px 11px;border:1px solid #c9bfaf;background:white;border-radius:7px;color:var(--ink)}button:hover{border-color:var(--accent)}button:focus-visible,a:focus-visible,textarea:focus-visible{outline:3px solid #a47634;outline-offset:2px}.primary{background:var(--accent);color:white;border-color:var(--accent)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.card{background:#fff;border:1px solid var(--line);border-radius:13px;padding:17px;scroll-margin-top:20px}.card[data-decision=approve]{border-color:#779a7b}.card[data-decision=revise]{border-color:#c69450}.card>header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin-bottom:12px}.id{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted)}.source{font-size:12px;color:var(--muted);text-align:right}.compare{display:grid;grid-template-columns:1fr 1fr;gap:11px}figure{margin:0;min-width:0}figcaption{font-size:11px;font-weight:650;color:var(--muted);min-height:20px}.frame{display:flex;align-items:center;justify-content:center;width:100%;height:220px;overflow:hidden;background:var(--frame);border-radius:8px}.frame img{display:block;max-width:100%;max-height:100%;object-fit:contain}.original{image-rendering:pixelated;width:auto;height:82%}.compare figure:nth-child(2) img{width:auto;height:82%}.game-size{display:flex;align-items:center;gap:12px;margin:10px 0 0;font-size:12px;color:var(--muted);min-height:82px}.game-size img{object-fit:contain}.choices{display:flex;gap:7px;flex-wrap:wrap;border-top:1px solid #eee9df;padding-top:13px;margin-top:14px}.choices button{font-size:12px;padding:6px 9px}.choices button[aria-pressed=true]{font-weight:700;background:#e9e0d1;border-color:var(--accent)}.choices button[data-decision=approve][aria-pressed=true]{background:#e3efe4;border-color:#779a7b}.choices button[data-decision=revise][aria-pressed=true]{background:#fff0d8;border-color:#c69450}textarea{display:block;width:100%;min-height:48px;resize:vertical;margin-top:10px;padding:8px 9px;border:1px solid var(--line);border-radius:6px;font:inherit;font-size:12px}details{border:1px solid var(--line);border-radius:10px;padding:13px 16px;background:#fffdf8;margin:20px 0}summary{cursor:pointer;font-weight:650}details p{font-size:13px;color:var(--muted)}.footer{font-size:12px;color:var(--muted);margin:22px 0}@media(max-width:870px){main{padding:24px 14px 100px}.grid{grid-template-columns:1fr}.frame{height:200px}h1{font-size:30px}.source{display:none}}@media(max-width:460px){.frame{height:145px}figcaption{font-size:10px}}
</style></head><body><main><div class="eyebrow">Clonk · graphics review</div><h1>Navigation arrows — batch 6</h1><p class="intro"><strong>Color and shading pass:</strong> the wooden right arrow now shares the left arrow's deep brown palette and light range. The red arrows retain a shared crimson finish and soft shadows. The four red 64×64 cells come from <code>Arrow.png</code>; the four 19×40 wooden cells are normal and pressed player-selection controls. The game assets have not been changed yet.</p><p><a href="overview.png?v=__OVERVIEW_VERSION__" target="_blank">View all eight before/afters in one image</a></p><div class="toolbar"><span id="progress">0 of 8 reviewed</span><button id="copy" class="primary" type="button">Copy review</button><button id="download" type="button">Download review JSON</button><span id="copy-status" aria-live="polite"></span></div><div class="grid">__CARDS__</div><details><summary>Other low-resolution candidates</summary><p><code>Rank.png</code> has twenty-four 16×16 badges; sixteen <code>Options.png</code> cells remain at 35×35. <code>Control.png</code> has mixed-size keyboard and command facets. <code>GUIContext.png</code> and <code>GUISubmenu.png</code> contain tiny wooden menu controls. <code>Flag.png</code> and <code>Crew.png</code> need owner-color handling.</p></details><p class="footer">Click an image for its full resolution. Selections stay in this browser until you copy or download them. Original: RedWolf Design, CC BY-NC 4.0; previews are adaptations.</p></main><script>
const cards=[...document.querySelectorAll('.card')],key='clonk-navigation-arrow-batch-6-review-v1';let answers={};try{answers=JSON.parse(localStorage.getItem(key)||'{}')||{}}catch{}const labels={approve:'Approve',revise:'Revise',keep:'Keep original'};function save(){try{localStorage.setItem(key,JSON.stringify(answers))}catch{}}function update(){let count=0;for(const card of cards){const answer=answers[card.dataset.id]||{};if(answer.decision)count++;card.dataset.decision=answer.decision||'';for(const button of card.querySelectorAll('button[data-decision]'))button.setAttribute('aria-pressed',String(button.dataset.decision===answer.decision));if(document.activeElement!==card.querySelector('textarea'))card.querySelector('textarea').value=answer.note||''}document.getElementById('progress').textContent=`${count} of ${cards.length} reviewed`}for(const card of cards){for(const button of card.querySelectorAll('button[data-decision]'))button.addEventListener('click',()=>{answers[card.dataset.id]={...answers[card.dataset.id],decision:button.dataset.decision};save();update()});card.querySelector('textarea').addEventListener('input',event=>{answers[card.dataset.id]={...answers[card.dataset.id],note:event.target.value};save()})}function result(){return cards.map(card=>({id:card.dataset.id,label:card.querySelector('h2').textContent,decision:answers[card.dataset.id]?.decision||'pending',note:answers[card.dataset.id]?.note||''}))}document.getElementById('copy').addEventListener('click',async()=>{const report='Clonk navigation arrow icon review — batch 6\\n\\n'+result().map(row=>`${row.id} ${row.label}: ${labels[row.decision]||'Pending'}${row.note?'\\n  '+row.note:''}`).join('\\n');try{await navigator.clipboard.writeText(report);document.getElementById('copy-status').textContent='Copied. Paste it into our chat.'}catch{const field=document.createElement('textarea');field.value=report;document.body.append(field);field.select();const ok=document.execCommand('copy');field.remove();document.getElementById('copy-status').textContent=ok?'Copied. Paste it into our chat.':'Copy unavailable; use Download review JSON.'}});document.getElementById('download').addEventListener('click',()=>{const blob=new Blob([JSON.stringify({batch:'Navigation arrows — batch 6',icons:result()},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='clonk-navigation-arrow-review-batch-6.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)});update();
</script></body></html>'''.replace("__CARDS__", "\n".join(cards)).replace(
    "__OVERVIEW_VERSION__", digest(HERE / "overview.png")[:12]
)
html = html.replace(
    "The game assets have not been changed yet.",
    "All eight are approved and installed as per-cell replacements; the original sheet geometry is unchanged.",
).replace(
    "key='clonk-navigation-arrow-batch-6-review-v1';let answers={};try",
    "key='clonk-navigation-arrow-batch-6-review-v2';let answers=Object.fromEntries(cards.map(card=>[card.dataset.id,{decision:'approve'}]));try",
)
(HERE / "index.html").write_text(html)
print(f"Built {len(icons)} review cards in {HERE}")
