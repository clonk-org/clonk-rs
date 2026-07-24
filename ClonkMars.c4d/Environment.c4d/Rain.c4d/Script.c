/*-- Regen --*/

#strict

protected func Initialize() {
  SetPosition(0,0);
  return(1);
}

private func Rain() {
  if(!GetPlayerCount()) return(0);
  
  var plr = 0;
  var range = 300;
  var borderleft = GetX(GetCursor(GetPlayerByIndex(plr)))-range;
  var borderright = GetX(GetCursor(GetPlayerByIndex(plr)))+range;

  for(; plr < GetPlayerCount(); plr++) {
    borderleft = Min(borderleft, GetX(GetCursor(GetPlayerByIndex(plr)))-range);
    borderright = Max(borderright, GetX(GetCursor(GetPlayerByIndex(plr)))+range);
  }
  
  CreateParticle("Raindrop", RandomX(borderleft, borderright), 0, RandomX(GetWind(0,0,1)*3, GetWind(0,0,1)*5), RandomX(200, 300), 5*64 + Random(32));
  return(1);
}
