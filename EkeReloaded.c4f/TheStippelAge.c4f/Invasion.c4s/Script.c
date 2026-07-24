/* Invasion */

#strict

static scientist;
static sciePanic;

func GetSftSuit() { return("Storm"); }

func Initialize() { ScriptGo(true); }

func InitializePlayer(player)
{
  var sft = GetCrew(player);
  var harpoon = CreateContents(HP5B, sft);

  CreateContents(TB5B, sft);
  CreateContents(FLAG, sft);
  CreateContents(NH5B, sft);

  var tent = CreateObject(TN5B, 2280, 305);

  scientist = CreateContents(CLNK, tent);
  SetColorDw(RGB(255, 255, 255), scientist);
  SetName("Prof. Meteo", scientist);
}

func RadioMessage(text)
{
  Message("<c ffff00><i>%s</i></c>", 0, text);
  Sound("SF_Radio", true);
}

func Script10() { RadioMessage("$Tent$"); }
func Script80()
{ 
  for(var i = 1; i <= 50; i++) CreateObject(CC5B, 2600 + (i * 10), 600);
  RadioMessage("$Scientist$");
}
func Script100() { RadioMessage("$Meeting$"); }
func Script101()
{ 
  var a1 = Find_ID(SF5B);
  var a2 = Find_OCF(OCF_Alive);
  var a3 = Find_Distance(50, GetX(scientist), GetY(scientist));

  if (!FindObject2(a1, a2, a3)) return(goto(101));

  Message("$Hello$", scientist);
  SetCommand(scientist, "MoveTo", 0, 2929, 710);
}
func Script120()
{
  var x = GetX(scientist);
  var y = GetY(scientist);
  
  var cocoon = FindObject2(Find_ID(CC5B), Find_Distance(25, x, y));
  if (cocoon) cocoon -> FxBirthTimer();
  
  if (sciePanic < 5)
  {
    var energy = GetPhysical("Energy", 0, scientist) / 1000;
    if (GetEnergy(scientist) == energy) Message("$Wonder$", scientist);
    else if (GetAlive(scientist)) Message("$Pain$", scientist);
    
    SetCommand(scientist, "MoveTo", 0, 2500 + Random(500), 700);
    sciePanic++;
    return(goto(102));
  }
  if (GetAlive(scientist)) Message("$Heart$", scientist);
}
func Script130() { if (GetAlive(scientist)) Kill(scientist); }

func Script150() { RadioMessage("$Alarm1$"); }
func Script160() { RadioMessage("$Alarm2$"); }
func Script170() { CreateObject(SE5B); }

func Script260() { RadioMessage("$Money1$"); }
func Script270() { RadioMessage("$Money2$"); }
func Script280() { RadioMessage("$Money3$"); }
func Script290()
{
  for (var i = 0; i < GetPlayerCount(C4PT_User); i++)
  {
    // Juhu, Geld ..und - juhu - ein sinnloser Kommentar
    DoWealth(GetPlayerByIndex(i, C4PT_User), 750);
  }
  Sound("Cash", true);
}
func Script370() { RadioMessage("$ArsoMorf1$"); }
func Script380() { RadioMessage("$ArsoMorf2$"); }
func Script390() { RadioMessage("$ArsoMorf3$"); }

func Script400() { if (ObjectCount(ST5B) > 150) return(goto(400)); }
func Script410() { RadioMessage("$Seal1$"); }
func Script420() { RadioMessage("$Seal2$"); }

func Script500() { if (ObjectCount(ST5B) > 50) return(goto(500)); }
func Script510() { RadioMessage("$Dinner1$"); }
func Script520() { RadioMessage("$Dinner2$"); }

func Script600() { if (ObjectCount(ST5B) > 10) return(goto(600)); }
func Script610() { RadioMessage("$Biomap1$"); }
func Script620() { RadioMessage("$Biomap2$"); }

func OnGameOver() { RadioMessage("$End$"); }