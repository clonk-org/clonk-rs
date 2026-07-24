/* SnowBase */

#strict

func GetSftSuit() { return("Snow"); }

func Initialize() 
{
  ScriptGo(1);
}

func Script1() { SetMaxPlayer(); }

func Script250()
{
  var goal = FindObject(CF5B);
  if (goal)
  {
    goal -> CreateAirbike(1, GetBunker3(), 125, 140, DIR_Right);
    goal -> CreateAirbike(2, GetBunker3(), 125, 140, DIR_Right);
  }
  goto(2);
}

func GetBunker1() { return(FindObject(DB5B, 0160, 676)); }
func GetBunker2() { return(FindObject(DB5B, 0986, 886)); }
func GetBunker3() { return(FindObject(DB5B, 1515, 226)); }
func GetBunker4() { return(FindObject(DB5B, 2342, 676)); }

func GetBunker1Name() { return("West Bunker"); }
func GetBunker2Name() { return("South Bunker"); }
func GetBunker3Name() { return("North Bunker"); }
func GetBunker4Name() { return("East Bunker"); }