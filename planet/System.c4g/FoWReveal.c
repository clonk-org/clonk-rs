/*-- Dragon Rock: lift a shadow before the Clonk is inside it --*/

// clonk-rs divergence from LegacyClonk. Dragon Rock is the only shipped
// content that uses a negative PlrViewRange as a persistent, map-authored
// shadow volume. Four _FOW generators darken the mountain, and because
// generators are applied after every repeller they override the Clonk's own
// light outright (C4Player::FoWGenerators2Map, C4Player.cpp:1949-1957). Each
// one also holds part of the interior in C4OS_INACTIVE -- 181 objects across
// the four -- and only gives them back when it removes itself
// (Fantasy.c4f/Drachenfels.c4s/FoWGenerator.c4d/Script.c:96-124).
//
// A generator lifts when CheckClonk finds a non-script-player crew member
// inside a search rect the map author saved with it, polled every 20 ticks by
// the Active row's NextAction=Active self-chain (ActMap.txt; the StartCall
// re-issue is C4Object.cpp:5480-5485). Those rects were authored per shadow
// and several of their edges fall *inside* the fully-black disc the same
// object generates. Distance from centre to each rect edge, against the dark
// radius, straight out of the shipped Objects.txt:
//
//   #2779  radius 235   left 320  right 280  top *160*  bottom *120*
//   #2781  radius 356   left 400  right *280*  top *280*  bottom *220*
//   #3835  radius 257   left *130*  right 300  top *200*  bottom *250*
//   #3905  radius 247   left 300  right *200*  top *100*  bottom *180*
//
// Every starred edge is nearer than the darkness it sits in. So approaching
// #3905 from above, a Clonk walks 147px through solid black before the area
// opens, and #2781 is late from three of its four sides. That is
// clonk-org/clonk-rs#214 -- the reveal reads as arriving after the fact,
// because it does.
//
// This append widens the trigger to the union of the authored rect and a
// circle matching the generator's own darkness plus a margin, so a shadow
// lifts as the Clonk reaches the edge of the black rather than after crossing
// it. Union, never replacement: the authored rect still triggers on its own,
// and it still reaches further than the circle at every one of its corners
// (#2779's is 358 against a 335 circle, #3905's 350 against 347), so no
// approach reveals later than it does today.
//
// Determinism contract: this file adds NO synchronized Random() draw and no
// new callback. It changes only the criterion CheckClonk hands FindObjects,
// on a definition that exists in exactly one scenario. It is nonetheless a
// *simulation* divergence, not a presentation one -- SetObjectStatus fires
// earlier than in C++ -- so it is recorded in PORT_STATUS.md, every peer
// stays in sync with every other peer because they all load this file, and
// Dragon Rock replays recorded before it will not reproduce.
//
// Deliberately NOT changed: the shipped rect itself, the 20-tick poll, the
// script-player exclusion, the GetController(o) > NO_OWNER guard, Activate,
// Deactivate, and the generated fog geometry. The darkness is exactly the
// size C++ draws it; only the moment it is taken away moves.

#strict

// `nowarn`: _FOW is defined inside Drachenfels.c4s, so every other scenario
// -- and every engine fixture that links System.c4g against a bare definition
// set -- has no such definition to append to (C4AulLink.cpp:42-49).
#appendto _FOW nowarn

// How far outside its own darkness a generator accepts a Clonk. The shadow
// fades out over a further 200px (FoWGenerators2Map passes
// -PlrViewRange + 200), so a Clonk standing here is inside the fade and can
// still see where it is going.
private func FoWRevealMargin() { return(100); }

// The dark radius this generator produces, inverted from the range it sets
// itself: SetPlrViewRange(Min((w+h)/-2+40, -1)) (Script.c:74,95). Both halves
// truncate toward zero, so this is exact and not an approximation of it. A
// generator that has not run ShadowInit yet has w = h = 0 and collapses to 1.
private func FoWRevealRadius() { return(Max((w+h)/2-40, 1) + FoWRevealMargin()); }

// Shipped CheckClonk (Script.c:114-124) with one criterion widened. Every
// other filter, the controller guard and the Deactivate/RemoveObject tail are
// reproduced exactly.
protected func CheckClonk()
{
  var o;
  for (o in FindObjects(Find_Or(Find_InRect(search_x, search_y, search_w, search_h),
                                Find_Distance(FoWRevealRadius())),
                        Find_OCF(OCF_CrewMember),
                        Find_NoContainer(),
                        Find_Not(Find_Owner(GetPlayerByIndex(0, C4PT_Script)))))
    if (GetController(o) > NO_OWNER)
      {
      Deactivate();
      return(RemoveObject());
      }
}
