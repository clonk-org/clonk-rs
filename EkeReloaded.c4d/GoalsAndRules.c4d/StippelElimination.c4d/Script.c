/* Stippelbekämpfung */

#strict
#include GOAL

func Initialize()
{
  if (!ObjectCount(ST5B))
  {
    var i = 10;
    while (i--) PlaceAnimal(ST5B);
  }
  return(inherited());  
}
    
func IsFulfilled()
{ 
  var b1 = !FindObject(ST5B);
  var b2 = !FindObject(MI5B);
  var b3 = !FindObject(MF5B);
  var b4 = !FindObject(MU5B);
  var b5 = !FindObject(CC5B);

  return(b1 && b2 && b3 && b4 && b5);
}

func Activate(player)
{
  if (IsFulfilled()) return(MessageWindow("$Done$", player));

  var stn = GetName(0, ST5B);
  var min = GetName(0, MI5B);
  var mfn = GetName(0, MF5B);
  var mun = GetName(0, MU5B);
  var ccn = GetName(0, CC5B);

  var stc = ObjectCount2(Find_ID(ST5B));
  var mic = ObjectCount2(Find_ID(MI5B));
  var mfc = ObjectCount2(Find_ID(MF5B));
  var muc = ObjectCount2(Find_ID(MU5B));
  var ccc = ObjectCount2(Find_ID(CC5B));

  var stm = Format("|• %s: %d", stn, stc);
  var mim = Format("|• %s: %d", min, mic);
  var mfm = Format("|• %s: %d", mfn, mfc);
  var mum = Format("|• %s: %d", mun, muc);
  var ccm = Format("|• %s: %d", ccn, ccc);

  var text = Format("<c ffaa77>Population</c>%s%s%s%s%s", stm, mim, mfm, mum, ccm);
  MessageWindow(text, player);
}