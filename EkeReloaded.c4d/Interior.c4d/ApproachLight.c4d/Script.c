/* Landelicht */

#strict

func Initialize()
{
  // Landelichter blinken in der Gruppe geordnet  
  var i = ObjectCount2(Find_ID(AL5B), Find_Distance(500));
  ScheduleCall(0, "StartBlinking", i * 5);
}

func StartBlinking()
{
  AddEffect("Blink", this, 1, 25, this);
}

func FxBlinkTimer()
{
  var x = 0;
  var y = -1;

  var size = 50; 
  var color = RGB(0, 255, 255);

  CreateParticle("EkePulse", x, y, 0, 0, size, color);
}