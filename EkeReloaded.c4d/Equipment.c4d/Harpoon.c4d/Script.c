/* Harpune */

#strict

local ammo;
local mode;
local rope;
local arrow;
local qGrenades;

local ropeCommand;
local ropeLength;
local canSwing;
local lastVertexNum;
local lastSwingDir;
local aSpeed;
local delay;

func Initialize()
{ 
  CreateContents(AW5B);

  ammo = 100;
  mode = "Anchor";
}

func ControlLeft(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  SetDir(DIR_Left(), clonk);
}

func ControlLeftDouble(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  StopAiming(clonk, COMD_Left());
}

func ControlRight(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  SetDir(DIR_Right(), clonk);
}

func ControlRightDouble(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  StopAiming(clonk, COMD_Right());
}

func ControlUp(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  ChangeAimAngle(clonk, -1); 
}

func ControlDown(clonk)
{
  if (GetAction(clonk) ne "HarpoonAim") return(0);
  ChangeAimAngle(clonk, +1);
}

func ControlThrow(clonk)
{ 
  if (rope) return(RemoveObject(rope));
  if (!WildcardMatch(GetAction(clonk), "Harpoon*")) return(1);
  if (!ammo) return(Sound("PT_Empty"));

  if (GetAction(clonk) eq "HarpoonWalk")
  {
    clonk -> SetAction("HarpoonAim");
    SetPhase(5, clonk);
  }
  else
  {
    var dir = GetDir(clonk);
    ShootArrow(dir, clonk);

    if (mode == "Anchor") ammo -= 100;
    else                  ammo -= 10;

    clonk -> SetAmmoBar(ammo);
    Sound("HP_Shoot");
    return(1);
  }
  return(1);
}

func ControlSpecial2(clonk)
{
  if (mode == "Anchor")
  {
    if (rope) return;
    ammo = qGrenades;
    mode = "Grenades";
  }
  else
  {
    qGrenades = ammo;
    ammo = 100;
    mode = "Anchor";
  }
  clonk -> ScrollHud(mode);
  clonk -> SetAmmoBar(ammo);
  return(1);
}

func Activate(object clonk)
{
  if (mode != "Grenades") return;  
  if (ammo == 100)        return;  

  var newAmmo;
  var reloadSound;

  var grenades = FindContents(GB5B, clonk);
  if (!grenades) return;
  RemoveObject(grenades);

  ammo = 100;
  clonk -> SetAmmoBar(ammo);

  Sound("HP_Reload");
  return(1);
}

func ShootArrow(dir, clonk)
{
  var x;
  var y;
  var xDir;
  var yDir;
  var followArrow;

  if (GetAction(clonk) eq "HarpoonAim")
  {
    var angle = (2 * dir - 1) * GetPhase(clonk) * 115 / 7;

    xDir = +Sin(angle, 12);
    yDir = -Cos(angle, 12);

    // Sicht folgt dem Pfeil
    followArrow = true;
  }
  else // aus dem Sprung
  {  
    xDir = GetXDir(clonk, 1) + (24 * dir - 12);
    yDir = GetYDir(clonk, 1);
  }
  if (mode == "Anchor")
  {
    arrow = Contents();
    rope = CreateObject(RP5B);
 
    arrow -> SetAction("Fly", clonk, this);
    rope -> Connect(this, arrow);
  }
  else
  {
    arrow = CreateContents(EA5B);
    arrow -> Launch(clonk);
    
    x = dir;
    y = 2 + (dir * 2) - (yDir / 4);
  }
  Exit(arrow, x, y, 0, xDir, yDir);
  if (followArrow) SetPlrView(GetOwner(clonk), arrow);
  return(1);
}

func Reeling()
{
  var clonk = Contained();

  // Clonk hängt an einem Seil einer anderen Harpune...
  var harpoon = FindObject2(Find_ID(HP5B),
                            Find_Action("Reel"),
                            Find_Container(clonk),
                            Find_Exclude(this()));

  // ...altes Seil trennen
  if (harpoon) return(RemoveObject(LocalN("rope", harpoon)));
 
  // Position vom Clonk
  var xPos = GetX(clonk);
  var yPos = GetY(clonk);

  // Geschwindigkeit vom Clonk
  var xDir = GetXDir(clonk);
  var yDir = GetYDir(clonk);

  // Drehpunkt vom Seil
  var xPiv = GetVertex(1, 0, rope); // X-Koordinate vom letzten Seil-Vertex
  var yPiv = GetVertex(1, 1, rope); // Y-Koordinate vom letzten Seil-Vertex

  var angle = Angle(xPiv, yPiv, xPos, yPos, 1000);

  var xAcc = 0;
  var yAcc = -2500;

  if (!ControlReady())
  {
    // Seillänge aktualisieren
    ropeLength = Distance(xPiv, yPiv, xPos, yPos);

    var ax = Cos(angle, xDir * 20000, 1000);
    var ay = Sin(angle, yDir * 20000, 1000);

    aSpeed = ax + ay;
    return(0);
  }
  // Seillänge zählt erst ab dem letzten Pivot-Point (Vertex)
  if (GetVertexNum(rope) != lastVertexNum)
  {
    ropeLength = Distance(xPiv, yPiv, xPos, yPos);
  }
  lastVertexNum = GetVertexNum(rope);

  // Schwingen freischalten
  var swingDir = xDir / Abs(xDir);
 
  var b1 = swingDir != lastSwingDir;
  var b2 = Abs(xDir) > 10;
  var b3 = Abs(xPiv - xPos) < 25; 

  //Message("@%d", this(), Abs(xPiv - xPos));

  if ((b1 && b2) || (!b2 && b3))
  {
    if (!ropeCommand) canSwing = true;
    lastSwingDir = swingDir;
  }
  if (ropeCommand == "ControlLeft")
  {
    if (canSwing)
    {
      // nach Links schwingen
      xAcc = -1;
      canSwing = false;
    }
    // Seilrolle stoppen
    ropeCommand = 0;
  }
  if (ropeCommand == "ControlRight")
  {
    if (canSwing)
    {
      // nach Rechts schwingen
      xAcc = 1;
      canSwing = false;
    }
    // Seilrolle stoppen
    ropeCommand = 0;
  } 

  // Seilrolle ansteuern
  if (ropeCommand == "ControlUp")
  {
    if (ropeLength > 2)
    { 
      ropeLength -= 2;
      yAcc = 5000;
      canSwing = false;
    }
  }
  if (ropeCommand == "ControlDown")
  {  
    if (ropeLength > 0)
    {
      ropeLength += 2;
      yAcc = -5000;
      canSwing = false;
    }
  }
  // Winkelgeschwindigkeit berechnen
  aSpeed += Sin(BoundBy(angle, 90000, 270000), 200000, 1000);
  aSpeed += Cos(BoundBy(angle, 90000, 270000), 900000, 1000) * xAcc;

  // Winkelgeschwindigkeit begrenzen
  aSpeed = BoundBy(aSpeed, -50000 * ropeLength, 50000 * ropeLength);

  // Dämpfung und Winkel ändern
  aSpeed += (aSpeed / Abs(aSpeed)) * yAcc;
  angle += aSpeed / ropeLength;

  // Überlauf vom Winkel verhindern
  if (angle > 360000) angle -= 360000;
  if (angle < 000000) angle += 360000;

  // Zielposition bestimmen
  var x = +Sin(angle, ropeLength, 1000) + xPiv;
  var y = -Cos(angle, ropeLength, 1000) + yPiv;

  // zeitlich begrenzten Gummi-Effekt hinzufügen
  var xOld = ((100 - delay) * xDir) / 100;
  var yOld = ((100 - delay) * yDir) / 100;

  var xNew = (delay * (x - xPos)) / 100;
  var yNew = (delay * (y - yPos)) / 100;

  // neue Geschwindigkeit vom Clonk setzen
  SetXDir(xOld + xNew, clonk);
  SetYDir(yOld + yNew, clonk);
}

func ControlReady()
{
  var clonk = Contained();

  delay += delay < 100;
  delay *= WildcardMatch(GetAction(clonk), "*Jump");

  if (delay > 1) return(1);

  var b1 = GetAction(clonk) eq "Tumble";
  var b2 = GetAction(clonk) eq "Dive";

  if (b1 || b2) return(clonk -> SetAction("Jump"));

  canSwing = false;
  return(0);
}

func BreakRope() // darf nur vom Seil aufgerufen werden!
{
  SetAction("Stop");
  delay = 0;

  CreateContents(AW5B);
  ammo = 100;
  
  var clonk = Contained();
  if (clonk) clonk ->~ SetAmmoBar(ammo); 

  Sound("HP_BreakRope");
  return(1);
}

func ChangeAimAngle(clonk, value)
{
  var angle = GetPhase(clonk);
  if (Inside(angle + value, 0, 7))
  {
    SetPhase(angle + value, clonk);
  }
}

func StopAiming(clonk, comDir)
{
  clonk -> SetComDir(comDir);
  clonk -> SetAction("HarpoonWalk");
  Sound("Grab");
}