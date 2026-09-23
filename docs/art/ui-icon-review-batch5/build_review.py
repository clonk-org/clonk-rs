"""Build the seven-pose Clonk hand icon review page."""

from __future__ import annotations

import hashlib
import json
from html import escape
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[2] / "planet/Graphics.c4g/Hand.png"
LABELS = (
    "Open grasp",
    "Closing grasp",
    "Flat hand",
    "Point",
    "Open horizontal hand",
    "Raised thumb",
    "Upright palm",
)
APPROVED = {1, 5, 6, 7}
REVISED = {2, 3, 4}
ROUND_ONE_FEEDBACK = "That doesn't look like a proper hand"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


icons = []
for phase, label in enumerate(LABELS):
    before = HERE / "before" / f"hand-{phase}.png"
    after = HERE / "after" / f"hand-{phase}.png"
    game_size = HERE / "game-size" / f"hand-{phase}.png"
    assert Image.open(before).size == (64, 64)
    assert Image.open(after).size == (512, 512)
    assert Image.open(game_size).size == (64, 64)
    icons.append(
        {
            "id": f"HAND-{phase + 1:02}",
            "label": label,
            "source": "planet/Graphics.c4g/Hand.png",
            "sourceCell": [phase * 64, 0, 64, 64],
            "sourceSha256": digest(SOURCE),
            "before": f"before/hand-{phase}.png",
            "after": f"after/hand-{phase}.png",
            "gameSize": f"game-size/hand-{phase}.png",
            "afterSize": [512, 512],
            "afterSha256": digest(after),
            "review": "approved",
            "roundOneReview": "approve" if phase + 1 in APPROVED else "revise",
            "roundOneFeedback": (
                None if phase + 1 in APPROVED else ROUND_ONE_FEEDBACK
            ),
            "previousPreview": (
                f"history/hand-{phase}-round-1.png"
                if phase + 1 in REVISED
                else None
            ),
            "roundTwoReview": "approve",
        }
    )

(HERE / "manifest.json").write_text(
    json.dumps(
        {
            "title": "Clonk hand gesture icon review — batch 5",
            "status": "All seven approved and installed as high-resolution Hand.png cell replacements",
            "method": "OpenAI built-in imagegen, checkerboard removal, and fitting to original alpha bounds",
            "icons": icons,
            "otherLowResolutionCandidates": [
                "Rank.png: twenty-four 16x16 rank symbols",
                "Options.png: sixteen remaining 35x35 cells",
                "Control.png: mixed-size keyboard and command facets",
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
    phase = icon["sourceCell"][0] // 64
    previous = (
        f'<p class="previous">Revised after round 1: {escape(ROUND_ONE_FEEDBACK)}. '
        f'<a href="{icon["previousPreview"]}" target="_blank">View previous proposal</a>. Approved in round 2.</p>'
        if phase + 1 in REVISED
        else '<p class="previous">Approved in round 1; artwork unchanged.</p>'
    )
    cards.append(
        f'''<article class="card" id="{identifier}" data-id="{identifier}">
  <header><div><span class="id">{identifier}</span><h2>{label}</h2></div><span class="source">Hand.png · phase {phase}</span></header>
  <div class="compare"><figure><figcaption>Original · 64×64</figcaption><a href="{icon['before']}" target="_blank"><span class="frame"><img class="original" src="{icon['before']}" alt="Original {label}"></span></a></figure><figure><figcaption>High-resolution preview · 512×512</figcaption><a href="{icon['after']}?v=2" target="_blank"><span class="frame"><img src="{icon['after']}?v=2" alt="Proposed {label}"></span></a></figure></div>
  <p class="game-size">At game size · 64×64: <img src="{icon['gameSize']}?v=2" width="96" height="96" alt="Proposed {label} at game size"></p>
  {previous}
  <div class="choices" role="group" aria-label="Review {label}"><button data-decision="approve" type="button">Approve</button><button data-decision="revise" type="button">Revise</button><button data-decision="keep" type="button">Keep original</button></div>
  <textarea aria-label="Revision notes for {label}" placeholder="Optional detail to preserve or change"></textarea>
</article>'''
    )

html = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clonk hand gestures — batch 5</title>
<style>
:root{--paper:#f5f2eb;--ink:#302d27;--muted:#716a60;--line:#ddd5c8;--accent:#75552b;--frame:#eae5da}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1420px;margin:auto;padding:34px 26px 100px}h1{font-size:38px;line-height:1.1;letter-spacing:-1px;margin:8px 0 12px}h2{font-size:19px;margin:1px 0}.eyebrow{font-size:12px;color:var(--accent);font-weight:700;text-transform:uppercase;letter-spacing:1.5px}.intro{max-width:960px;color:var(--muted);margin:0 0 18px}.intro strong{color:var(--ink)}.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;background:#fffdf8;border:1px solid var(--line);border-radius:10px;padding:11px 15px;margin:20px 0}.toolbar span:first-child{margin-right:auto;font-weight:650}button{font:inherit;cursor:pointer;padding:7px 11px;border:1px solid #c9bfaf;background:white;border-radius:7px;color:var(--ink)}button:hover{border-color:var(--accent)}button:focus-visible,a:focus-visible,textarea:focus-visible{outline:3px solid #a47634;outline-offset:2px}.primary{background:var(--accent);color:white;border-color:var(--accent)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.card{background:#fff;border:1px solid var(--line);border-radius:13px;padding:17px;scroll-margin-top:20px}.card[data-decision=approve]{border-color:#779a7b}.card[data-decision=revise]{border-color:#c69450}.card>header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin-bottom:12px}.id{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted)}.source{font-size:12px;color:var(--muted);text-align:right}.compare{display:grid;grid-template-columns:1fr 1fr;gap:11px}figure{margin:0;min-width:0}figcaption{font-size:11px;font-weight:650;color:var(--muted);min-height:20px}.frame{display:flex;align-items:center;justify-content:center;width:100%;height:220px;overflow:hidden;background:var(--frame);border-radius:8px}.frame img{display:block;width:100%;height:100%;object-fit:contain}.original{image-rendering:pixelated}.game-size{display:flex;align-items:center;gap:12px;margin:10px 0 0;font-size:12px;color:var(--muted)}.game-size img{object-fit:contain}.previous{font-size:12px;color:var(--muted);margin:10px 0 0}.choices{display:flex;gap:7px;flex-wrap:wrap;border-top:1px solid #eee9df;padding-top:13px;margin-top:14px}.choices button{font-size:12px;padding:6px 9px}.choices button[aria-pressed=true]{font-weight:700;background:#e9e0d1;border-color:var(--accent)}.choices button[data-decision=approve][aria-pressed=true]{background:#e3efe4;border-color:#779a7b}.choices button[data-decision=revise][aria-pressed=true]{background:#fff0d8;border-color:#c69450}textarea{display:block;width:100%;min-height:48px;resize:vertical;margin-top:10px;padding:8px 9px;border:1px solid var(--line);border-radius:6px;font:inherit;font-size:12px}details{border:1px solid var(--line);border-radius:10px;padding:13px 16px;background:#fffdf8;margin:20px 0}summary{cursor:pointer;font-weight:650}details p{font-size:13px;color:var(--muted)}.footer{font-size:12px;color:var(--muted);margin:22px 0}@media(max-width:870px){main{padding:24px 14px 100px}.grid{grid-template-columns:1fr}.frame{height:200px}h1{font-size:30px}.source{display:none}}@media(max-width:460px){.frame{height:145px}figcaption{font-size:10px}}
</style></head><body><main><div class="eyebrow">Clonk · graphics review</div><h1>Hand gestures — batch 5</h1><p class="intro"><strong>All seven gestures are approved and installed. HAND-02, HAND-03, and HAND-04 were revised for more natural hand anatomy.</strong> Compare each with its original at review size and game size. The original Hand.png sheet remains intact; the app renders these higher-resolution cells over the stock sheet.</p><p><a href="overview.png?v=2" target="_blank">View all seven before/afters</a> · <a href="revision-comparison.png?v=2" target="_blank">Compare the three revisions with round 1</a></p><div class="toolbar"><span id="progress">7 of 7 reviewed</span><button id="copy" class="primary" type="button">Copy review</button><button id="download" type="button">Download review JSON</button><span id="copy-status" aria-live="polite"></span></div><div class="grid">__CARDS__</div><details><summary>Other low-resolution candidates</summary><p><code>Rank.png</code> has twenty-four 16×16 symbols; sixteen <code>Options.png</code> cells remain at 35×35. <code>Control.png</code> has mixed-size keyboard and command facets. <code>Flag.png</code> and <code>Crew.png</code> need their owner-color masks preserved.</p></details><p class="footer">Click an image for its full resolution. Selections stay in this browser until you copy or download them. Original: RedWolf Design, CC BY-NC 4.0; previews are adaptations.</p></main><script>
const cards=[...document.querySelectorAll('.card')],key='clonk-hand-icon-batch-5-review-v3';let answers={'HAND-01':{decision:'approve'},'HAND-05':{decision:'approve'},'HAND-06':{decision:'approve'},'HAND-07':{decision:'approve'},'HAND-02':{decision:'approve'},'HAND-03':{decision:'approve'},'HAND-04':{decision:'approve'}};try{answers={...answers,...(JSON.parse(localStorage.getItem(key)||'{}')||{})}}catch{}const labels={approve:'Approve',revise:'Revise',keep:'Keep original'};function save(){try{localStorage.setItem(key,JSON.stringify(answers))}catch{}}function update(){let count=0;for(const card of cards){const answer=answers[card.dataset.id]||{};if(answer.decision)count++;card.dataset.decision=answer.decision||'';for(const button of card.querySelectorAll('button[data-decision]'))button.setAttribute('aria-pressed',String(button.dataset.decision===answer.decision));if(document.activeElement!==card.querySelector('textarea'))card.querySelector('textarea').value=answer.note||''}document.getElementById('progress').textContent=`${count} of ${cards.length} reviewed`}for(const card of cards){for(const button of card.querySelectorAll('button[data-decision]'))button.addEventListener('click',()=>{answers[card.dataset.id]={...answers[card.dataset.id],decision:button.dataset.decision};save();update()});card.querySelector('textarea').addEventListener('input',event=>{answers[card.dataset.id]={...answers[card.dataset.id],note:event.target.value};save()})}function result(){return cards.map(card=>({id:card.dataset.id,label:card.querySelector('h2').textContent,decision:answers[card.dataset.id]?.decision||'pending',note:answers[card.dataset.id]?.note||''}))}document.getElementById('copy').addEventListener('click',async()=>{const report='Clonk hand gesture icon review — batch 5\\n\\n'+result().map(row=>`${row.id} ${row.label}: ${labels[row.decision]||'Pending'}${row.note?'\\n  '+row.note:''}`).join('\\n');try{await navigator.clipboard.writeText(report);document.getElementById('copy-status').textContent='Copied. Paste it into our chat.'}catch{const field=document.createElement('textarea');field.value=report;document.body.append(field);field.select();const ok=document.execCommand('copy');field.remove();document.getElementById('copy-status').textContent=ok?'Copied. Paste it into our chat.':'Copy unavailable; use Download review JSON.'}});document.getElementById('download').addEventListener('click',()=>{const blob=new Blob([JSON.stringify({batch:'Hand gestures — batch 5',icons:result()},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='clonk-hand-icon-review-batch-5.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)});update();
</script></body></html>'''.replace("__CARDS__", "\n".join(cards))
(HERE / "index.html").write_text(html)
print(f"Built {len(icons)} review cards in {HERE}")
