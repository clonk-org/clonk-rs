"""Build the self-contained second UI artwork review from source PNGs."""
from __future__ import annotations

import hashlib
import json
import shutil
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
PREVIEW_SCALES = {
    "build": 1.07,
    "captain": 1.12,
    "construction": 1.0,
    "energy": 1.0,
    "exit": 1.04,
    "magic": 1.0,
    "player": 1.0,
    "score": 1.15,
    "wealth": 1.55,
}

ITEMS = [
    ("HUD-01", "build", "Build", "Build tool", "Build command symbol"),
    ("HUD-02", "captain", "Captain", "Captain star", "Captain indicator"),
    ("HUD-03", "construction", "Construction", "Construction hammer", "Construction indicator"),
    ("HUD-04", "energy", "Energy", "Red energy bolt", "Object energy indicator"),
    ("HUD-05", "exit", "Exit", "Open door", "Exit command symbol"),
    ("HUD-06", "magic", "Magic", "Blue magic bolt", "Spell menu value"),
    ("HUD-07", "player", "Player", "Player portrait", "Player information"),
    ("HUD-08", "score", "Score", "Score buildings", "Score display"),
    ("HUD-09", "wealth", "Wealth", "Gold pile", "Wealth display"),
]

ROUND_ONE_APPROVED = {"build", "exit", "magic", "score", "wealth"}
APPROVED = {item[1] for item in ITEMS}
REVISIONS = {
    "captain": "Flatter gold star with a thin offset shadow.",
    "construction": "A proper two-faced mallet with a centered wooden shaft.",
    "energy": "The approved blue bolt, recolored red without changing its shape.",
    "player": "Rebuilt using the game's portrait of the same Clonk as reference.",
}
GAME_REFERENCES = {
    "construction": ("reference/construction-game-mallet.png", "Game mallet reference"),
    "player": ("reference/player-canonical-portrait.png", "Game Clonk portrait reference"),
}


def png_size(path: Path) -> list[int]:
    with path.open("rb") as stream:
        header = stream.read(24)
    assert header[:8] == b"\x89PNG\r\n\x1a\n", path
    return list(struct.unpack(">II", header[16:24]))


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


icons = []
(HERE / "before").mkdir(exist_ok=True)
for identifier, key, original_name, label, usage in ITEMS:
    source = ROOT / "planet" / "Graphics.c4g" / f"{original_name}.png"
    before = HERE / "before" / f"{key}.png"
    candidate = HERE / "after" / f"{key}.png"
    assert source.is_file() and candidate.is_file(), (source, candidate)
    shutil.copyfile(source, before)
    source_size = png_size(source)
    candidate_size = png_size(candidate)
    assert candidate_size[0] >= 512 and candidate_size[1] >= 512, candidate
    icons.append({
        "id": identifier,
        "key": key,
        "label": label,
        "usage": usage,
        "source": f"planet/Graphics.c4g/{original_name}.png",
        "sourceSize": source_size,
        "sourceSha256": sha256(source),
        "before": f"before/{key}.png",
        "candidate": f"after/{key}.png",
        "candidateSize": candidate_size,
        "candidateSha256": sha256(candidate),
        "previewScale": PREVIEW_SCALES[key],
        "review": "approved" if key in APPROVED else "pending",
        "roundOneReview": "approved" if key in ROUND_ONE_APPROVED else "revise",
        "revision": REVISIONS.get(key),
        "priorCandidate": f"history/{key}-round-1.png" if key in REVISIONS else None,
        "gameReference": GAME_REFERENCES.get(key, (None,))[0],
    })
manifest = {
    "title": "Clonk HUD icon review — batch 2",
    "generator": "OpenAI built-in imagegen; red bolt recolored from the approved blue bolt",
    "status": "All nine approved; transparent high-resolution assets integrated into the game",
    "previewBackground": "Opaque generated backdrop; transparent game sprites prepared in crates/clonk-app/assets/hud-icons",
    "icons": icons,
    "otherFindings": [
        "Flag.png and Crew.png need owner-color treatment",
        "Liquid.png is an animation texture rather than a pictorial icon",
        "Menu.png, Options.png, Rank.png, Gamepad.png and StartupScenSelIcons.png are multi-cell sheets for later review",
    ],
}
(HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

cards = []
for icon in icons:
    source_size = "×".join(map(str, icon["sourceSize"]))
    candidate_size = "×".join(map(str, icon["candidateSize"]))
    reference = GAME_REFERENCES.get(icon["key"])
    reference_link = (f" <a href=\"{reference[0]}\" target=\"_blank\">{reference[1]}</a>"
                      if reference else "")
    review_note = (f"<p class=\"feedback approved\"><strong>Approved after revision:</strong> {icon['revision']} "
                   f"<a href=\"{icon['priorCandidate']}\" target=\"_blank\">Earlier preview</a>"
                   f"{reference_link}</p>"
                   if icon["revision"] else "<p class=\"feedback approved\">Approved in round one</p>")
    cards.append(f'''<article class="card" id="{icon['id']}" data-id="{icon['id']}" data-prior="{'approve' if icon['key'] in APPROVED else ''}">
      <header><div><span class="id">{icon['id']}</span><h2>{icon['label']}</h2></div><span class="usage">{icon['usage']}</span></header>
      <div class="compare"><figure><figcaption>Original · {source_size}</figcaption><a href="{icon['before']}" target="_blank"><span class="frame"><img class="original" src="{icon['before']}" alt="Original {icon['label']}"></span></a></figure><figure><figcaption>Super-resolution preview · {candidate_size}</figcaption><a href="{icon['candidate']}" target="_blank"><span class="frame"><img style="transform:scale({icon['previewScale']})" src="{icon['candidate']}" alt="Generated {icon['label']}"></span></a></figure></div>
      {review_note}
      <div class="choices" role="group" aria-label="Review {icon['label']}"><button data-decision="approve" type="button">Approve</button><button data-decision="revise" type="button">Revise</button><button data-decision="keep" type="button">Keep original</button></div>
      <textarea aria-label="Revision notes for {icon['label']}" placeholder="Optional detail to preserve or change"></textarea>
    </article>''')

html = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clonk HUD icon review — batch 2</title>
<style>
:root{--paper:#f5f2eb;--ink:#302d27;--muted:#716a60;--line:#ddd5c8;--accent:#75552b;--frame:#eee8dc}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1420px;margin:auto;padding:34px 26px 100px}h1{font-size:38px;line-height:1.1;letter-spacing:-1px;margin:8px 0 12px}h2{font-size:19px;margin:1px 0}.eyebrow{font-size:12px;color:var(--accent);font-weight:700;text-transform:uppercase;letter-spacing:1.5px}.intro{max-width:850px;color:var(--muted);margin:0 0 18px}.intro strong{color:var(--ink)}.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;background:#fffdf8;border:1px solid var(--line);border-radius:10px;padding:11px 15px;margin:20px 0}.toolbar span{margin-right:auto;font-weight:650}button{font:inherit;cursor:pointer;padding:7px 11px;border:1px solid #c9bfaf;background:white;border-radius:7px;color:var(--ink)}button:hover{border-color:var(--accent)}button:focus-visible,a:focus-visible,textarea:focus-visible{outline:3px solid #a47634;outline-offset:2px}.primary{background:var(--accent);color:white;border-color:var(--accent)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.card{background:#fff;border:1px solid var(--line);border-radius:13px;padding:17px;scroll-margin-top:20px}.card[data-decision=approve]{border-color:#779a7b}.card[data-decision=revise]{border-color:#c69450}.card>header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;margin-bottom:12px}.id{font:12px ui-monospace,SFMono-Regular,monospace;color:var(--muted)}.usage{font-size:12px;color:var(--muted);text-align:right}.compare{display:grid;grid-template-columns:1fr 1fr;gap:11px}figure{margin:0;min-width:0}figcaption{font-size:11px;font-weight:650;color:var(--muted);min-height:20px}.frame{display:flex;align-items:center;justify-content:center;width:100%;height:240px;overflow:hidden;background:var(--frame);border-radius:8px}.frame img{display:block;width:100%;height:100%;object-fit:contain}.original{image-rendering:pixelated}.choices{display:flex;gap:7px;flex-wrap:wrap;border-top:1px solid #eee9df;padding-top:13px;margin-top:14px}.choices button{font-size:12px;padding:6px 9px}.choices button[aria-pressed=true]{font-weight:700;background:#e9e0d1;border-color:var(--accent)}.choices button[data-decision=approve][aria-pressed=true]{background:#e3efe4;border-color:#779a7b}.choices button[data-decision=revise][aria-pressed=true]{background:#fff0d8;border-color:#c69450}textarea{display:block;width:100%;min-height:48px;resize:vertical;margin-top:10px;padding:8px 9px;border:1px solid var(--line);border-radius:6px;font:inherit;font-size:12px}.note{font-size:12px;color:var(--muted);margin:22px 0}details{border:1px solid var(--line);border-radius:10px;padding:13px 16px;background:#fffdf8;margin:20px 0}summary{cursor:pointer;font-weight:650}details p{font-size:13px;color:var(--muted)}@media(max-width:870px){main{padding:24px 14px 100px}.grid{grid-template-columns:1fr}.frame{height:210px}h1{font-size:30px}.usage{display:none}}@media(max-width:460px){.frame{height:170px}figcaption{font-size:10px}}
</style></head><body><main><div class="eyebrow">Clonk · graphics review</div><h1>HUD icon review — batch 2</h1><p class="intro"><strong>Nine low-resolution game symbols have new artwork previews.</strong> Compare each one with the enlarged original, then choose Approve, Revise, or Keep original. These generated previews have opaque backgrounds; approved art will be prepared as transparent game sprites before integration.</p><div class="toolbar"><span id="progress">0 of 9 reviewed</span><button id="copy" class="primary" type="button">Copy review</button><button id="download" type="button">Download review JSON</button><span id="copy-status" aria-live="polite"></span></div><div class="grid">__CARDS__</div><details><summary>Why these nine?</summary><p>These are standalone pictorial symbols used by the game. Flag and Crew also carry owner colors, which need a separate treatment. Liquid is an animation texture. The multi-cell Menu, Options, Rank, Gamepad, and scenario-icon sheets will be reviewed in their own batches.</p></details><p class="note">Click an image to inspect it at its full source size. Review selections stay in this browser until you copy or download them.</p></main><script>
const cards=[...document.querySelectorAll('.card')];const key='clonk-hud-icon-batch-2-review';let answers={};try{answers=JSON.parse(localStorage.getItem(key)||'{}')||{}}catch{}const labels={approve:'Approve',revise:'Revise',keep:'Keep original'};function save(){try{localStorage.setItem(key,JSON.stringify(answers))}catch{}}function update(){let count=0;for(const card of cards){const answer=answers[card.dataset.id]||{};if(answer.decision)count++;card.dataset.decision=answer.decision||'';for(const button of card.querySelectorAll('button[data-decision]'))button.setAttribute('aria-pressed',String(button.dataset.decision===answer.decision));if(document.activeElement!==card.querySelector('textarea'))card.querySelector('textarea').value=answer.note||''}document.getElementById('progress').textContent=`${count} of ${cards.length} reviewed`}for(const card of cards){for(const button of card.querySelectorAll('button[data-decision]'))button.addEventListener('click',()=>{answers[card.dataset.id]={...answers[card.dataset.id],decision:button.dataset.decision};save();update()});card.querySelector('textarea').addEventListener('input',event=>{answers[card.dataset.id]={...answers[card.dataset.id],note:event.target.value};save()})}function result(){return cards.map(card=>({id:card.dataset.id,label:card.querySelector('h2').textContent,decision:answers[card.dataset.id]?.decision||'pending',note:answers[card.dataset.id]?.note||''}))}document.getElementById('copy').addEventListener('click',async()=>{const report='Clonk HUD icon review — batch 2\\n\\n'+result().map(row=>`${row.id} ${row.label}: ${labels[row.decision]||'Pending'}${row.note?'\\n  '+row.note:''}`).join('\\n');try{await navigator.clipboard.writeText(report);document.getElementById('copy-status').textContent='Copied. Paste it into our chat.'}catch{const field=document.createElement('textarea');field.value=report;document.body.append(field);field.select();const ok=document.execCommand('copy');field.remove();document.getElementById('copy-status').textContent=ok?'Copied. Paste it into our chat.':'Copy unavailable; use Download review JSON.'}});document.getElementById('download').addEventListener('click',()=>{const blob=new Blob([JSON.stringify({batch:'HUD icons — batch 2',icons:result()},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob);const link=document.createElement('a');link.href=url;link.download='clonk-hud-icon-review-batch-2.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)});update();
</script></body></html>'''.replace('__CARDS__', '\n'.join(cards))
html = html.replace(
    "<strong>Nine low-resolution game symbols have new artwork previews.</strong> Compare each one with the enlarged original, then choose Approve, Revise, or Keep original. These generated previews have opaque backgrounds; approved art will be prepared as transparent game sprites before integration.",
    "<strong>All nine HUD icons are approved and integrated into the game.</strong> Compare the original art with the approved super-resolution previews. The previews retain their generated backdrop; the game assets have transparent backgrounds and preserve the original HUD layout.",
)
html = html.replace("0 of 9 reviewed", "9 of 9 approved")
html = html.replace(
    "clonk-hud-icon-batch-2-review",
    "clonk-hud-icon-batch-2-final-review",
)
html = html.replace("Review selections stay in this browser until you copy or download them.", "The approval record is in history/review-round-2.json.")
html = html.replace(
    "catch{}const labels=",
    "catch{}for(const card of cards){if(card.dataset.prior&&!answers[card.dataset.id])"
    "answers[card.dataset.id]={decision:card.dataset.prior}}const labels=",
)
html = html.replace(
    "</style>",
    ".feedback{min-height:38px;margin:10px 0 0;font-size:12px;color:#855824}"
    ".feedback a{margin-left:5px}.feedback.approved{color:#4b7b50}"
    "</style>",
)
(HERE / 'index.html').write_text(html)
print(f'Built {len(icons)} review cards in {HERE}')
