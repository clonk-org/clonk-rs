"""Build the self-contained third Clonk icon review from extracted source cells."""

from __future__ import annotations

import hashlib
import json
import struct
from html import escape
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CELL = 35
ITEMS = [
    ("MENU-01", "menu-save", "Save game", "Menu.png", 0, "Uses the already-approved UI-05 Save artwork."),
    ("MENU-02", "menu-goals", "Goals", "Menu.png", 4, "Restored the brown backdrop and tall gray tower behind the stone building and gold."),
    ("MENU-03", "menu-rules", "Rules", "Menu.png", 5, "Square gold plaque with a blue flag, red lightning bolt, and gray hammer."),
    ("MENU-04", "menu-hostility", "Hostility", "Menu.png", 7, "Crossed swords now have broad, flatter Clonk-style blades."),
    ("MENU-05", "menu-display", "Display", "Menu.png", 8, "Plain CRT monitor; paired with the FPS icon below."),
    ("MENU-06", "options-settings", "Settings", "Options.png", 0, "The brass gear teeth now engage at their contact point."),
    ("MENU-07", "options-music", "Music", "Options.png", 1, "Two connected golden notes."),
    ("MENU-08", "options-fps", "FPS display", "Options.png", 5, "The same CRT monitor as Display, with FPS on its screen."),
    ("MENU-09", "options-audio", "Audio", "Options.png", 17, "A golden speaker cone informed by the game's StartupOptionIconsHD speaker."),
]

ROUND_ONE = {
    "MENU-01": ("approve", ""),
    "MENU-02": ("revise", "The background and the tower structure are missing from the high-res version"),
    "MENU-03": ("revise", "The flag doesn't have a pole in the high res version"),
    "MENU-04": ("revise", "Flatten the swords, make sure we're preserving the Clonk style"),
    "MENU-05": ("approve", ""),
    "MENU-06": ("revise", "Gears aren't meshing"),
    "MENU-07": ("approve", ""),
    "MENU-08": ("approve", ""),
    "MENU-09": ("revise", "This is supposed to be a speaker, not an ear"),
}

ROUND_TWO = {
    "MENU-01": ("approve", ""),
    "MENU-02": ("approve", ""),
    "MENU-03": ("pending", "This doesn't look like the original. The square edges aren't chamfered, the blue flag and the lightning bolt and the hammer aren't represented well."),
    "MENU-04": ("approve", ""),
    "MENU-05": ("approve", ""),
    "MENU-06": ("approve", ""),
    "MENU-07": ("approve", ""),
    "MENU-08": ("approve", ""),
    "MENU-09": ("approve", ""),
}

ROUND_THREE = {
    "MENU-03": ("pending", "The HD version should NOT have chamfered edges, but the contents inside look correct now"),
}

ROUND_FOUR = {identifier: ("approve", "") for identifier, *_ in ITEMS}


def png_size(path: Path) -> list[int]:
    header = path.read_bytes()[:24]
    assert header[:8] == b"\x89PNG\r\n\x1a\n", path
    return list(struct.unpack(">II", header[16:24]))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


icons = []
for identifier, key, label, sheet, phase, note in ITEMS:
    before = HERE / "before" / f"{key}.png"
    after = HERE / "after" / f"{key}.png"
    assert before.is_file() and after.is_file(), key
    assert png_size(before) == [CELL, CELL], before
    icons.append(
        {
            "id": identifier,
            "key": key,
            "label": label,
            "source": f"planet/Graphics.c4g/{sheet}",
            "sourceCell": [phase * CELL, 0, CELL, CELL],
            "sourceSha256": digest(ROOT / "planet" / "Graphics.c4g" / sheet),
            "before": f"before/{key}.png",
            "after": f"after/{key}.png",
            "afterSize": png_size(after),
            "afterSha256": digest(after),
            "review": "approved",
            "roundOneReview": ROUND_ONE[identifier][0],
            "roundOneFeedback": ROUND_ONE[identifier][1],
            "roundTwoReview": ROUND_TWO[identifier][0],
            "roundTwoFeedback": ROUND_TWO[identifier][1],
            "roundThreeReview": ROUND_THREE.get(identifier, (None, None))[0],
            "roundThreeFeedback": ROUND_THREE.get(identifier, (None, None))[1],
            "roundFourReview": ROUND_FOUR[identifier][0],
            "previousPreview": f"history/{key}-round-3.png" if identifier == "MENU-03" else (f"history/{key}-round-1.png" if ROUND_ONE[identifier][0] == "revise" else None),
            "note": note,
        }
    )

manifest = {
    "title": "Clonk in-game menu and options icon review — batch 3",
    "status": "All nine previews approved and integrated in the game at 8x; three checked Options variants use the approved high-resolution checkmark",
    "method": "OpenAI built-in imagegen, except Save game, which reuses approved UI-05 art",
    "previewBackground": "Hostility and Settings have generated ivory review backdrops; prepare_runtime.sh removes them and fits all approved art to 280x280 transparent sprites.",
    "icons": icons,
    "otherLowResolutionCandidates": [
        "Other Options.png cells and toggle pairs (35x35 cells)",
        "Gamepad.png four 80x36 controller phases",
        "Hand.png seven 64x64 gesture phases",
        "Rank.png 24 16x16 symbols",
        "Flag.png and Crew.png, which require owner-color treatment",
    ],
}
(HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
(HERE / "history" / "review-round-1.json").write_text(
    json.dumps({identifier: {"decision": decision, "feedback": feedback} for identifier, (decision, feedback) in ROUND_ONE.items()}, indent=2) + "\n"
)
(HERE / "history" / "review-round-2.json").write_text(
    json.dumps({identifier: {"decision": decision, "feedback": feedback} for identifier, (decision, feedback) in ROUND_TWO.items()}, indent=2) + "\n"
)
(HERE / "history" / "review-round-3.json").write_text(
    json.dumps({identifier: {"decision": decision, "feedback": feedback} for identifier, (decision, feedback) in ROUND_THREE.items()}, indent=2) + "\n"
)
(HERE / "history" / "review-round-4.json").write_text(
    json.dumps({identifier: {"decision": decision, "feedback": feedback} for identifier, (decision, feedback) in ROUND_FOUR.items()}, indent=2) + "\n"
)

cards = []
for icon in icons:
    identifier = escape(icon["id"])
    label = escape(icon["label"])
    source = escape(icon["source"].rsplit("/", 1)[-1])
    after_size = "×".join(map(str, icon["afterSize"]))
    state = f'Approved in round {4 if identifier == "MENU-03" else (1 if icon["roundOneReview"] == "approve" else 2)}'
    feedback_round = 3 if identifier == "MENU-03" else 1
    feedback = icon["roundThreeFeedback"] if feedback_round == 3 else icon["roundOneFeedback"]
    previous = (
        f'<p class="previous">Round {feedback_round} feedback: {escape(feedback)} '
        f'<a href="{icon["previousPreview"]}" target="_blank">View previous preview</a></p>'
        if icon["previousPreview"] else ""
    )
    cards.append(
        f'''<article class="card" id="{identifier}" data-id="{identifier}" data-review="{icon['review']}">
  <header><div><span class="id">{identifier}</span><h2>{label}</h2><span class="badge">{state}</span></div><span class="source">{source} · cell {icon['sourceCell'][0] // CELL}</span></header>
  <div class="compare"><figure><figcaption>Original · 35×35</figcaption><a href="{icon['before']}" target="_blank"><span class="frame"><img class="original" src="{icon['before']}" alt="Original {label}"></span></a></figure><figure><figcaption>High-resolution preview · {after_size}</figcaption><a href="{icon['after']}" target="_blank"><span class="frame"><img src="{icon['after']}" alt="Proposed {label}"></span></a></figure></div>
  <p class="note">{escape(icon['note'])}</p>
{previous}
  <div class="choices" role="group" aria-label="Review {label}"><button data-decision="approve" type="button">Approve</button><button data-decision="revise" type="button">Revise</button><button data-decision="keep" type="button">Keep original</button></div>
  <textarea aria-label="Revision notes for {label}" placeholder="Optional detail to preserve or change"></textarea>
</article>'''
    )

html = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clonk menu icon review — batch 3</title>
<style>
:root{--paper:#f5f2eb;--ink:#302d27;--muted:#716a60;--line:#ddd5c8;--accent:#75552b;--frame:#eee8dc}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1420px;margin:auto;padding:34px 26px 100px}h1{font-size:38px;line-height:1.1;letter-spacing:-1px;margin:8px 0 12px}h2{font-size:19px;margin:1px 0}.eyebrow{font-size:12px;color:var(--accent);font-weight:700;text-transform:uppercase;letter-spacing:1.5px}.intro{max-width:900px;color:var(--muted);margin:0 0 18px}.intro strong{color:var(--ink)}.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;background:#fffdf8;border:1px solid var(--line);border-radius:10px;padding:11px 15px;margin:20px 0}.toolbar span{margin-right:auto;font-weight:650}button{font:inherit;cursor:pointer;padding:7px 11px;border:1px solid #c9bfaf;background:white;border-radius:7px;color:var(--ink)}button:hover{border-color:var(--accent)}button:focus-visible,a:focus-visible,textarea:focus-visible{outline:3px solid #a47634;outline-offset:2px}.primary{background:var(--accent);color:white;border-color:var(--accent)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.card{background:#fff;border:1px solid var(--line);border-radius:13px;padding:17px;scroll-margin-top:20px}.card[data-decision=approve]{border-color:#779a7b}.card[data-decision=revise]{border-color:#c69450}.card>header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin-bottom:12px}.id{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted)}.source{font-size:12px;color:var(--muted);text-align:right}.compare{display:grid;grid-template-columns:1fr 1fr;gap:11px}figure{margin:0;min-width:0}figcaption{font-size:11px;font-weight:650;color:var(--muted);min-height:20px}.frame{display:flex;align-items:center;justify-content:center;width:100%;height:240px;overflow:hidden;background:var(--frame);border-radius:8px}.frame img{display:block;width:100%;height:100%;object-fit:contain}.original{image-rendering:pixelated}.note{min-height:30px;margin:10px 0 0;font-size:12px;color:#6f5d45}.previous{margin:4px 0 0;font-size:12px;color:var(--muted)}.previous a{color:var(--accent)}.badge{display:inline-block;margin-top:2px;padding:1px 6px;border-radius:5px;background:#e9efe8;color:#496c4d;font-size:11px;font-weight:700}.card[data-review=pending] .badge{background:#fff0d8;color:#8b602a}.choices{display:flex;gap:7px;flex-wrap:wrap;border-top:1px solid #eee9df;padding-top:13px;margin-top:14px}.choices button{font-size:12px;padding:6px 9px}.choices button[aria-pressed=true]{font-weight:700;background:#e9e0d1;border-color:var(--accent)}.choices button[data-decision=approve][aria-pressed=true]{background:#e3efe4;border-color:#779a7b}.choices button[data-decision=revise][aria-pressed=true]{background:#fff0d8;border-color:#c69450}textarea{display:block;width:100%;min-height:48px;resize:vertical;margin-top:10px;padding:8px 9px;border:1px solid var(--line);border-radius:6px;font:inherit;font-size:12px}.footer{font-size:12px;color:var(--muted);margin:22px 0}details{border:1px solid var(--line);border-radius:10px;padding:13px 16px;background:#fffdf8;margin:20px 0}summary{cursor:pointer;font-weight:650}details p{font-size:13px;color:var(--muted)}@media(max-width:870px){main{padding:24px 14px 100px}.grid{grid-template-columns:1fr}.frame{height:210px}h1{font-size:30px}.source{display:none}}@media(max-width:460px){.frame{height:170px}figcaption{font-size:10px}}
</style></head><body><main><div class="eyebrow">Clonk · graphics review</div><h1>In-game menu icons — batch 3</h1><p class="intro"><strong>All nine icons were approved and are integrated into the game.</strong> The Rules plaque has square outer corners and retains the approved blue flag, red lightning bolt, and gray hammer. The runtime sprites have transparent backgrounds and preserve the original menu layout. Music, FPS, and Audio also have matching high-resolution checked states.</p><div class="toolbar"><span id="progress">0 of 9 reviewed</span><button id="copy" class="primary" type="button">Copy review</button><button id="download" type="button">Download review JSON</button><span id="copy-status" aria-live="polite"></span></div><div class="grid">__CARDS__</div><details><summary>What is still low resolution?</summary><p>Other Options cells, Gamepad controller phases, Hand gestures, Rank symbols, and owner-colored Flag and Crew art are also candidates. They need their own review batches so paired states and owner colors remain consistent.</p></details><p class="footer">Click an image to inspect its full source size. Your selections stay in this browser until you copy or download them.</p></main><script>
const cards=[...document.querySelectorAll('.card')];const key='clonk-menu-icon-batch-3-review-v5';let answers=Object.fromEntries(cards.map(card=>[card.dataset.id,{decision:'approve'}]));try{answers={...answers,...(JSON.parse(localStorage.getItem(key)||'{}')||{})}}catch{}const labels={approve:'Approve',revise:'Revise',keep:'Keep original'};function save(){try{localStorage.setItem(key,JSON.stringify(answers))}catch{}}function update(){let count=0;for(const card of cards){const answer=answers[card.dataset.id]||{};if(answer.decision)count++;card.dataset.decision=answer.decision||'';for(const button of card.querySelectorAll('button[data-decision]'))button.setAttribute('aria-pressed',String(button.dataset.decision===answer.decision));if(document.activeElement!==card.querySelector('textarea'))card.querySelector('textarea').value=answer.note||''}document.getElementById('progress').textContent=`${count} of ${cards.length} reviewed`}for(const card of cards){for(const button of card.querySelectorAll('button[data-decision]'))button.addEventListener('click',()=>{answers[card.dataset.id]={...answers[card.dataset.id],decision:button.dataset.decision};save();update()});card.querySelector('textarea').addEventListener('input',event=>{answers[card.dataset.id]={...answers[card.dataset.id],note:event.target.value};save()})}function result(){return cards.map(card=>({id:card.dataset.id,label:card.querySelector('h2').textContent,decision:answers[card.dataset.id]?.decision||'pending',note:answers[card.dataset.id]?.note||''}))}document.getElementById('copy').addEventListener('click',async()=>{const report='Clonk in-game menu icon review — batch 3\\n\\n'+result().map(row=>`${row.id} ${row.label}: ${labels[row.decision]||'Pending'}${row.note?'\\n  '+row.note:''}`).join('\\n');try{await navigator.clipboard.writeText(report);document.getElementById('copy-status').textContent='Copied. Paste it into our chat.'}catch{const field=document.createElement('textarea');field.value=report;document.body.append(field);field.select();const ok=document.execCommand('copy');field.remove();document.getElementById('copy-status').textContent=ok?'Copied. Paste it into our chat.':'Copy unavailable; use Download review JSON.'}});document.getElementById('download').addEventListener('click',()=>{const blob=new Blob([JSON.stringify({batch:'In-game menu icons — batch 3',icons:result()},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='clonk-menu-icon-review-batch-3.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)});update();
</script></body></html>'''.replace('__CARDS__', '\n'.join(cards))
(HERE / "index.html").write_text(html)
print(f"Built {len(icons)} review cards in {HERE}")
