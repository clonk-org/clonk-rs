/* Schalter */

#strict

func Initialize()
{
  AddEffect("Blink", this, 1, 10, this);
}

func FxBlinkTimer()
{
  var x = +6;
  var y = -6;

  var size = 50; 
  var color = RGB(255, 0, 0);
  
  if (GetAction() eq "Green")
  {
    y += 3;    
    color = RGB(0, 255, 0);
  }
  CreateParticle("EkePulse", x, y, 0, 0, size, color);
}

func ControlDig()
{
  // Standard-Funktion des Schalters: Gate öffnen
  var gate = FindObject2(Find_ID(GT5B), Sort_Distance());
  
  if (GetAction() eq "Green")
  {
    gate -> Close();
    SetAction("Red");
  }
  else
  {
    gate -> Open();
    SetAction("Green");
  }
  Sound("SI_Switch");
}