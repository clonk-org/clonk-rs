"""Build the self-contained fourth Clonk icon review."""

from __future__ import annotations

import hashlib
import json
import struct
from html import escape
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SOURCE = ROOT / "planet/Graphics.c4g/Gamepad.png"
ROUND_ONE_FEEDBACK = "The numbers need to be more faithful"
ROUND_TWO_FEEDBACK = "Those numbers look scuffed"
ROUND_THREE_APPROVAL = "This is great!"
ROUND_THREE_FEEDBACK = "The 3 could be a little better though"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_size(path: Path) -> list[int]:
    header = path.read_bytes()[:24]
    assert header[:8] == b"\x89PNG\r\n\x1a\n", path
    return list(struct.unpack(">II", header[16:24]))


icons = []
for number in range(1, 5):
    before = HERE / f"before/gamepad-{number}.png"
    after = HERE / f"after/gamepad-{number}.png"
    assert png_size(before) == [80, 36]
    assert png_size(after) == [1280, 576]
    icons.append(
        {
            "id": f"PAD-{number:02}",
            "label": f"Player {number} gamepad",
            "source": "planet/Graphics.c4g/Gamepad.png",
            "sourceCell": [(number - 1) * 80, 0, 80, 36],
            "sourceSha256": digest(SOURCE),
            "before": f"before/gamepad-{number}.png",
            "after": f"after/gamepad-{number}.png",
            "gameSize": f"game-size/gamepad-{number}.png",
            "afterSize": [1280, 576],
            "afterSha256": digest(after),
            "review": "approved",
            "roundOneReview": "revise",
            "roundOneFeedback": ROUND_ONE_FEEDBACK,
            "roundTwoReview": "revise",
            "roundTwoFeedback": ROUND_TWO_FEEDBACK,
            "roundThreeReview": "revise" if number == 3 else "approve",
            "roundThreeFeedback": (
                ROUND_THREE_FEEDBACK if number == 3 else ROUND_THREE_APPROVAL
            ),
            "roundFourReview": "approve" if number == 3 else None,
            "previousPreview": f"history/gamepad-{number}-round-{3 if number == 3 else 2}.png",
            "revisionDetail": "pad-03-revision-detail.png" if number == 3 else None,
            "roundOnePreview": f"history/gamepad-{number}-round-1.png",
            "note": (
                "The 3 has a flatter top, clearer waist, and more balanced lower curve; the controller is unchanged."
                if number == 3
                else "Unchanged from round 3; the approved controller base is unchanged."
            ),
        }
    )

manifest = {
    "title": "Clonk gamepad icon review — batch 4",
    "status": "All four approved; installed as high-resolution Gamepad.png cell replacements",
    "method": "OpenAI built-in imagegen controller and polished numeral edits, with deterministic transparent cutouts and original-cell placement",
    "controllerBaseSha256": digest(HERE / "support/controller-base.png"),
    "icons": icons,
    "otherLowResolutionCandidates": [
        "Hand.png: seven 64x64 gesture phases",
        "Rank.png: twenty-four 16x16 rank symbols",
        "Options.png: sixteen 35x35 cells outside the seven upgraded phases",
        "Control.png: mixed keyboard, command and key facets",
        "Flag.png and Crew.png: owner-colored art requiring color-mask preservation",
    ],
}
(HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
(HERE / "history/review-round-1.json").write_text(
    json.dumps({icon["id"]: {"decision": "revise", "feedback": ROUND_ONE_FEEDBACK} for icon in icons}, indent=2) + "\n"
)
(HERE / "history/review-round-2.json").write_text(
    json.dumps({icon["id"]: {"decision": "revise", "feedback": ROUND_TWO_FEEDBACK} for icon in icons}, indent=2) + "\n"
)
(HERE / "history/review-round-3.json").write_text(
    json.dumps(
        {
            icon["id"]: {
                "decision": icon["roundThreeReview"],
                "feedback": icon["roundThreeFeedback"],
            }
            for icon in icons
        },
        indent=2,
    )
    + "\n"
)
(HERE / "history/review-round-4.json").write_text(
    json.dumps({"PAD-03": {"decision": "approve", "feedback": "This is good, ship it"}}, indent=2) + "\n"
)

cards = []
for icon in icons:
    identifier = escape(icon["id"])
    label = escape(icon["label"])
    previous = (
        f'''Round 3 feedback: {escape(icon['roundThreeFeedback'])}. <a href="{icon['revisionDetail']}" target="_blank">Compare 3s side by side</a> · <a href="{icon['previousPreview']}" target="_blank">View previous 3</a>'''
        if identifier == "PAD-03"
        else f'''Approved from round 3. <a href="{icon['previousPreview']}" target="_blank">View round 2</a>'''
    )
    cards.append(
        f'''<article class="card" id="{identifier}" data-id="{identifier}">
  <header><div><span class="id">{identifier}</span><h2>{label}</h2></div><span class="source">Gamepad.png · phase {icon['sourceCell'][0] // 80}</span></header>
  <div class="compare"><figure><figcaption>Original · 80×36</figcaption><a href="{icon['before']}" target="_blank"><span class="frame"><img class="original" src="{icon['before']}" alt="Original {label}"></span></a></figure><figure><figcaption>High-resolution preview · 1280×576</figcaption><a href="{icon['after']}?v=4" target="_blank"><span class="frame"><img src="{icon['after']}?v=4" alt="Proposed {label}"></span></a></figure></div>
  <p class="game-size">At game size · 80×36: <img src="{icon['gameSize']}?v=4" width="160" height="72" alt="Revised {label} at game size"></p>
  <p class="note">{escape(icon['note'])}</p>
  <p class="previous">{previous}</p>
  <div class="choices" role="group" aria-label="Review {label}"><button data-decision="approve" type="button">Approve</button><button data-decision="revise" type="button">Revise</button><button data-decision="keep" type="button">Keep original</button></div>
  <textarea aria-label="Revision notes for {label}" placeholder="Optional detail to preserve or change"></textarea>
</article>'''
    )

html = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clonk gamepad icon review — batch 4</title>
<style>
:root{--paper:#f5f2eb;--ink:#302d27;--muted:#716a60;--line:#ddd5c8;--accent:#75552b;--frame:#eee8dc}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1420px;margin:auto;padding:34px 26px 100px}h1{font-size:38px;line-height:1.1;letter-spacing:-1px;margin:8px 0 12px}h2{font-size:19px;margin:1px 0}.eyebrow{font-size:12px;color:var(--accent);font-weight:700;text-transform:uppercase;letter-spacing:1.5px}.intro{max-width:960px;color:var(--muted);margin:0 0 18px}.intro strong{color:var(--ink)}.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;background:#fffdf8;border:1px solid var(--line);border-radius:10px;padding:11px 15px;margin:20px 0}.toolbar span{margin-right:auto;font-weight:650}button{font:inherit;cursor:pointer;padding:7px 11px;border:1px solid #c9bfaf;background:white;border-radius:7px;color:var(--ink)}button:hover{border-color:var(--accent)}button:focus-visible,a:focus-visible,textarea:focus-visible{outline:3px solid #a47634;outline-offset:2px}.primary{background:var(--accent);color:white;border-color:var(--accent)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.card{background:#fff;border:1px solid var(--line);border-radius:13px;padding:17px;scroll-margin-top:20px}.card[data-decision=approve]{border-color:#779a7b}.card[data-decision=revise]{border-color:#c69450}.card>header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin-bottom:12px}.id{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted)}.source{font-size:12px;color:var(--muted);text-align:right}.compare{display:grid;grid-template-columns:1fr 1fr;gap:11px}figure{margin:0;min-width:0}figcaption{font-size:11px;font-weight:650;color:var(--muted);min-height:20px}.frame{display:flex;align-items:center;justify-content:center;width:100%;height:190px;overflow:hidden;background:var(--frame);border-radius:8px}.frame img{display:block;width:100%;height:100%;object-fit:contain}.original{image-rendering:pixelated}.game-size{display:flex;align-items:center;gap:12px;margin:10px 0 0;font-size:12px;color:var(--muted)}.game-size img{image-rendering:pixelated;object-fit:contain}.note{margin:10px 0 0;font-size:12px;color:#6f5d45}.choices{display:flex;gap:7px;flex-wrap:wrap;border-top:1px solid #eee9df;padding-top:13px;margin-top:14px}.choices button{font-size:12px;padding:6px 9px}.choices button[aria-pressed=true]{font-weight:700;background:#e9e0d1;border-color:var(--accent)}.choices button[data-decision=approve][aria-pressed=true]{background:#e3efe4;border-color:#779a7b}.choices button[data-decision=revise][aria-pressed=true]{background:#fff0d8;border-color:#c69450}textarea{display:block;width:100%;min-height:48px;resize:vertical;margin-top:10px;padding:8px 9px;border:1px solid var(--line);border-radius:6px;font:inherit;font-size:12px}details{border:1px solid var(--line);border-radius:10px;padding:13px 16px;background:#fffdf8;margin:20px 0}summary{cursor:pointer;font-weight:650}details p{font-size:13px;color:var(--muted)}.footer{font-size:12px;color:var(--muted);margin:22px 0}@media(max-width:870px){main{padding:24px 14px 100px}.grid{grid-template-columns:1fr}.frame{height:180px}h1{font-size:30px}.source{display:none}}@media(max-width:460px){.frame{height:130px}figcaption{font-size:10px}}
</style></head><body><main><div class="eyebrow">Clonk · graphics review</div><h1>Gamepad icons — batch 4</h1><p class="intro"><strong>All four gamepad phases are approved and installed.</strong> The final player 3 numeral has a flatter top, a clearer opening at its waist, and a more even lower curve. Players 1, 2, and 4 are unchanged from round 3. The controller artwork is byte-for-byte unchanged. Compare the approved art with the original 80×36 cells; earlier versions remain linked below each card.</p><div class="toolbar"><span id="progress">4 of 4 reviewed</span><button id="copy" class="primary" type="button">Copy review</button><button id="download" type="button">Download review JSON</button><span id="copy-status" aria-live="polite"></span></div><div class="grid">__CARDS__</div><details><summary>Other low-resolution candidates</summary><p><code>Hand.png</code> has seven 64×64 gestures; <code>Rank.png</code> has twenty-four 16×16 symbols. Sixteen other 35×35 <code>Options.png</code> cells and mixed-size <code>Control.png</code> facets remain. <code>Flag.png</code> and <code>Crew.png</code> need owner-color handling before replacement.</p></details><p class="footer">Click an image to inspect its full source size. Selections stay in this browser until you copy or download them.</p></main><script>
const cards=[...document.querySelectorAll('.card')];const key='clonk-gamepad-icon-batch-4-review-v5';let answers={'PAD-01':{decision:'approve'},'PAD-02':{decision:'approve'},'PAD-03':{decision:'approve'},'PAD-04':{decision:'approve'}};try{answers={...answers,...(JSON.parse(localStorage.getItem(key)||'{}')||{})}}catch{}const labels={approve:'Approve',revise:'Revise',keep:'Keep original'};function save(){try{localStorage.setItem(key,JSON.stringify(answers))}catch{}}function update(){let count=0;for(const card of cards){const answer=answers[card.dataset.id]||{};if(answer.decision)count++;card.dataset.decision=answer.decision||'';for(const button of card.querySelectorAll('button[data-decision]'))button.setAttribute('aria-pressed',String(button.dataset.decision===answer.decision));if(document.activeElement!==card.querySelector('textarea'))card.querySelector('textarea').value=answer.note||''}document.getElementById('progress').textContent=`${count} of ${cards.length} reviewed`}for(const card of cards){for(const button of card.querySelectorAll('button[data-decision]'))button.addEventListener('click',()=>{answers[card.dataset.id]={...answers[card.dataset.id],decision:button.dataset.decision};save();update()});card.querySelector('textarea').addEventListener('input',event=>{answers[card.dataset.id]={...answers[card.dataset.id],note:event.target.value};save()})}function result(){return cards.map(card=>({id:card.dataset.id,label:card.querySelector('h2').textContent,decision:answers[card.dataset.id]?.decision||'pending',note:answers[card.dataset.id]?.note||''}))}document.getElementById('copy').addEventListener('click',async()=>{const report='Clonk gamepad icon review — batch 4\\n\\n'+result().map(row=>`${row.id} ${row.label}: ${labels[row.decision]||'Pending'}${row.note?'\\n  '+row.note:''}`).join('\\n');try{await navigator.clipboard.writeText(report);document.getElementById('copy-status').textContent='Copied. Paste it into our chat.'}catch{const field=document.createElement('textarea');field.value=report;document.body.append(field);field.select();const ok=document.execCommand('copy');field.remove();document.getElementById('copy-status').textContent=ok?'Copied. Paste it into our chat.':'Copy unavailable; use Download review JSON.'}});document.getElementById('download').addEventListener('click',()=>{const blob=new Blob([JSON.stringify({batch:'Gamepad icons — batch 4',icons:result()},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='clonk-gamepad-icon-review-batch-4.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)});update();
</script></body></html>'''.replace('__CARDS__', '\n'.join(cards))
(HERE / "index.html").write_text(html)
print(f"Built {len(icons)} review cards in {HERE}")
