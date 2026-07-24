/* Bürolicht */

#strict

local light;
local flickering;

func Initialize()
{
  light = CreateObject(NL5B, 0, 0, NO_OWNER);
  light -> SetAction("Shine", this);
}

func Damage()
{
  if (GetDamage() > 0)
  {
    light -> SetClrModulation(0x11FFFFFF);
    Sound("OL_Hum1", 0, 0, 25, 0, +1);
  }
  if (GetDamage() > 10)
  {
    // leichtes Brummen ausschalten
    Sound("OL_Hum1", 0, 0, 25, 0, -1);
    
    // ab jetzt läuft Schaden-Management per FX
    AddEffect("Flicker", this, 1, Random(50) + 1, this);
  }
}

func FxFlickerTimer()
{
  if (!flickering)
  {
    Sound("OL_Hum2", 0, 0, 10, 0, 1);

    ChangeEffect("Flicker", this, 0, "Flicker", 1);
    flickering = true;
  }
  var alpha = Random(50);
  light -> SetClrModulation(RGBa(255, 255, 255, alpha));

  if (!Random(25))
  {
    light -> SetClrModulation(0x00FFFFFF);
    Sound("OL_Hum2", 0, 0, 10, 0, -1);
  
    ChangeEffect("Flicker", this, 0, "Flicker", Random(50) + 1);
    flickering = false;
  }
  if (GetDamage() > 50)
  {
    // jetzt ist die Lampe ganz hinüber
    light -> SetClrModulation(0xFFFFFFFF);
    Sound("OL_Hum2", 0, 0, 10, 0, -1);
    
    return(-1);
  }
}