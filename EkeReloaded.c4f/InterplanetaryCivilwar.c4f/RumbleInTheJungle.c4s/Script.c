/* Rumble in the Jungle */

#strict

func GetSftSuit() { return("Jungle"); }

func Initialize() 
{
  SetSkyParallax(0, 14, 30);
  
  PlaceVegetations(BUSH, 25);
  PlaceVegetations(PLM1, 75);
  PlaceVegetations(PLM2, 50);
  PlaceVegetations(TRE3, 50);

  ScriptGo(1);
}

func Script1() { SetMaxPlayer(); }

func Script250()
{
  var goal = FindObject(CF5B);
  if (goal)
  {
    goal -> CreateAirbike(1, GetBunker1(), 140, 0, DIR_Right);
    goal -> CreateAirbike(2, GetBunker4(), -160, 0, DIR_Left);
  }
  goto(2);
}

func PlaceVegetations(id idTree, int iCount)
{
  var x = 0;
  var y = 0;

  var wdt = LandscapeWidth();
  var hgt = LandscapeHeight();

  for (var i = 0; i < iCount; ++i)
  {
    PlaceVegetation(idTree, x, y, wdt, hgt, 100000);
  }
}

func GetBunker1() { return(FindObject(DB5B, 0161, 486)); }
func GetBunker2() { return(FindObject(DB5B, 1089, 206)); }
func GetBunker3() { return(FindObject(DB5B, 1663, 796)); }
func GetBunker4() { return(FindObject(DB5B, 2462, 486)); }

func GetBunker1Name() { return("West Bunker"); }
func GetBunker2Name() { return("North Bunker"); }
func GetBunker3Name() { return("South Bunker"); }
func GetBunker4Name() { return("East Bunker"); }