/* SFT */

#strict

#include CLNK

local pistol;

protected func Initialize()
{
  // Kontextmenü-Regel aktivieren
  if (!ObjectCount(CM5B)) CreateObject(CM5B, 0, 0, NO_OWNER);

  // damit alte SFTs das Jetpack benutzen können
  SetPhysical("Float", 200, 1);

  // Munition der integrierten Pistole aufladen
  AddEffect("SellCheck", this, 1, 35, this());
  
  // weiter mit überladener Funktion
  return(inherited());
}

protected func Recruitment()
{
  // das Szenario bestimmt den Look
  var suit = GameCall("GetSftSuit", this);
  if (suit)
  {
    SetGraphics(suit);
    SetPortrait(Format("%s1", suit));
  }  
  // richtigen Werte gleich zu Beginn
  SetAmmoBar(LocalN("ammo", Contents()));

  // weiter mit überladener Funktion
  return(inherited());
}

protected func ControlLeft()
{
  // im Sprung mit/ohne Waffe...
  if (WildcardMatch(GetAction(), "*Jump"))
  {
    // ggf. Richtung ändern
    if (GetDir() != DIR_Left) SetDir(DIR_Left);
  }
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlLeft")) return(1);

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlLeft")) return(1);

  // Steuerung an Harpune weitergeben
  Control2Harpoon("ControlLeft");

  // Keine überladene Steuerung
  return(0);
}

protected func ControlLeftDouble()
{  
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlLeftDouble")) return(1);

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlLeftDouble")) return(1); 
  
  // Keine überladene Steuerung
  return(0);
}

protected func ControlRight()
{
  // im Sprung mit/ohne Waffe...
  if (WildcardMatch(GetAction(), "*Jump"))
  {
    // ggf. Richtung ändern
    if (GetDir() != DIR_Right) SetDir(DIR_Right);
  }
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlRight")) return(1);

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlRight")) return(1);

  // Steuerung an Harpune weitergeben
  Control2Harpoon("ControlRight");

  // Keine überladene Steuerung
  return(0);
}

protected func ControlRightDouble()
{
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlRightDouble")) return(1); 

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlRightDouble")) return(1); 
  
  // Keine überladene Steuerung
  return(0);
}

protected func ControlUp()
{
  // Schlag mit dem Kopf beim Nahkampf
  if (GetAction() eq "Fight")
  {
    SetAction("Headbang");
    return(1);
  }
  // Aufsitzen aufs Airbike immer erlauben
  if (CanGetOnAirbike()) return(1);

  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlUp")) return(1);
  
  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlUp")) return(1);

  // Steuerung an Harpune weitergeben
  Control2Harpoon("ControlUp");

  // Keine überladene Steuerung
  return(0);
}

protected func ControlDown()
{
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlDown")) return(1);

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlDown")) return(1);

  // Steuerung an Harpune weitergeben
  Control2Harpoon("ControlDown");

  // Keine überladene Steuerung
  return(0);
}

protected func ControlDownDouble()
{
  // Steuerung an Harpune weitergeben
  //Control2Harpoon("ControlDownDouble");

  // Keine überladene Steuerung
  return(0);
}

protected func ControlThrow()
{
  // Schlagen beim Nahkampf
  if (GetAction() eq "Fight")
  {
    SetAction("Punch");
    return(1);
  }
  // Bei vorherigem Doppel-Stop nur Ablegen   
  if (GetPlrDownDouble(GetOwner())) return(0);

  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlThrow")) return(1); 

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlThrow")) return(1); 
 
  // Keine überladene Steuerung
  return(0);
}

protected func ControlThrowDouble()
{
  ControlThrow();
  return(1);
}

protected func ControlDig()
{
  // Kicken beim Nahkampf
  if (GetAction() eq "Fight")
  {
    SetAction("Kick");
    return(1);
  }
  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlDig")) return(1);

  // Keine überladene Steuerung
  return(0);
}

protected func ControlDigSingle()
{
  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlDigSingle")) return(1); 

  // Keine überladene Steuerung
  return(0);
}

protected func ControlDigDouble()
{
  if (WildcardMatch(GetAction(), "*Walk")) return(0);
  if (WildcardMatch(GetAction(), "*Swim")) return(0);
  if (GetAction() eq "Dig")                return(0);
  if (GetAction() eq "Push")               return(0);
  if (GetAction() eq "Kick")               return(0);

  // Steuerung an Airbike weitergeben
  if (Control2Airbike("Activate")) return(1);

  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("Activate")) return(1); 

  // Keine überladene Steuerung
  return(0);
}

protected func ControlSpecial()
{  
  // ggf. Pistole wegstecken
  if (FindContents(PT5B))
  {
    Holster();
    return(0);
  }
  // Inventar verschieben
  ShiftContents(0, 0, 0, 1);
  
  // Bewaffnung prüfen
  CheckArmed();
  return(1);
}

protected func ControlSpecialDouble()
{
  ControlSpecial();
  return(1);
}

protected func ControlSpecial2()
{
  // Steuerung an Airbike weitergeben
  if (Control2Airbike("ControlSpecial2"))
  {
    Sound("SF_Mode");
    return(1);
  }
  // Steuerung an Inhaltsobjekt weitergeben
  if (Control2Contents("ControlSpecial2"))
  {
    Sound("SF_Mode");
    return(1);
  }
  // bei angefassten Objekten Holen ermöglichen
  if (GetAction() eq "Push")
  {
    var stuff = GetActionTarget();
    if (GetDefCoreVal("GrabPutGet", "DefCore", GetID(stuff)) > 1)
    {
      SetCommand(0, "Get", GetActionTarget(), 0, 0, 0, 1);
      return(1);
    }
  }
  Holster();
  return(0);
}

protected func ControlSpecial2Double()
{
  ControlSpecial2();
  return(1);
}

private func Control2Airbike(string command)
{ 
  // sitzt der SFT überhaupt auf einem Airbike?
  if (GetAction() ne "AirbikeFly") return(0);

  var result = ObjectCall(GetActionTarget(), command, this());  
  return(result);
}

private func Control2Contents(string command)
{
  // angefasste Objekte haben Vorrang, außer bei Modus-Auswahl
  if ((GetAction() eq "Push") && (command != "ControlSpecial2")) return(0);

  var result = ObjectCall(Contents(), command, this());
  return(result);
}

private func Control2Harpoon(string command)
{
  var harpoon = FindObject2(Find_Container(this()), Find_ID(HP5B), Find_Action("Reel"));
  if (harpoon) LocalN("ropeCommand", harpoon) = command;
}

protected func ContactLeft()
{
  // nicht weiter bei Kontakt am Spielfeldrand
  if (!GetContact(0, -1, CNAT_Left)) return(0);

  FallDamage();
  
  if (GetAction() eq "JetpackFly")
  {
    SetComDir(COMD_Stop());
    SetAction("Scale");
  }
  return(1);
}

protected func ContactRight()
{
  // nicht weiter bei Kontakt am Spielfeldrand
  if (!GetContact(0, -1, CNAT_Right)) return(0);  

  FallDamage();
  
  if (GetAction() eq "JetpackFly")
  {
    SetComDir(COMD_Stop());
    SetAction("Scale");
  }
  return(1);
}

protected func ContactTop()
{
  FallDamage();
  
  if (GetAction() eq "JetpackFly")
  {
    SetComDir(COMD_Stop());
    SetAction("Hangle");
  }
  return(1);
}

protected func ContactBottom()
{
  FallDamage();
  
  if (GetAction() eq "JetpackFly")
  {
    SetComDir(COMD_Stop());
    SetAction("JetpackWalk");
  }
  return(1);
}

private func CanGetOnAirbike()
{
  var airbike = FindObject2(Find_ID(AB5B),
                            Find_Distance(10),
                            Find_NoContainer());

  if (!airbike)                             return(0);
  if (GetAction() eq "Push")                return(0);
  if (!(airbike -> ControlRequest(this()))) return(0);
  
  return(1);
}

private func FallDamage()
{
  // weiter nur wenn der Clonk noch lebt
  if(!GetAlive()) return(0);

  var speed = Max(Abs(GetXDir()), Abs(GetYDir()));

  if (speed >= 70)
  {
    DoEnergy((70 - speed) * 2);
    Sound("Hurt*");
  } 
  return(1);
}

private func CheckArmed()
{  
  if (GetAction() eq "AirbikeFly") return(0);

  ScrollHud(LocalN("mode", Contents()));
  SetAmmoBar(LocalN("ammo", Contents()));

  if (GetAction() eq "Walk") if (GetID(Contents()) == PT5B) return(SetActionSmooth("PistolWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == PT5B) return(SetActionSmooth("PistolJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == PT5B) return(SetActionSmooth("PistolJump"));
  if (GetAction() eq "PistolWalk") if (GetID(Contents()) != PT5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "PistolJump") if (GetID(Contents()) != PT5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == UZ5B) return(SetActionSmooth("UziWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == UZ5B) return(SetActionSmooth("UziJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == UZ5B) return(SetActionSmooth("UziJump"));
  if (GetAction() eq "UziWalk") if (GetID(Contents()) != UZ5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "UziJump") if (GetID(Contents()) != UZ5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == FT5B) return(SetActionSmooth("FlamethrowerWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == FT5B) return(SetActionSmooth("FlamethrowerJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == FT5B) return(SetActionSmooth("FlamethrowerJump"));
  if (GetAction() eq "FlamethrowerWalk") if (GetID(Contents()) != FT5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "FlamethrowerJump") if (GetID(Contents()) != FT5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == AR5B) return(SetActionSmooth("AssaultRifleWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == AR5B) return(SetActionSmooth("AssaultRifleJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == AR5B) return(SetActionSmooth("AssaultRifleJump"));
  if (GetAction() eq "AssaultRifleWalk") if (GetID(Contents()) != AR5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "AssaultRifleJump") if (GetID(Contents()) != AR5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == RL5B) return(SetActionSmooth("RocketLauncherWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == RL5B) return(SetActionSmooth("RocketLauncherJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == RL5B) return(SetActionSmooth("RocketLauncherJump"));
  if (GetAction() eq "RocketLauncherWalk") if (GetID(Contents()) != RL5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "RocketLauncherJump") if (GetID(Contents()) != RL5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == SG5B) return(SetActionSmooth("ShotgunWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == SG5B) return(SetActionSmooth("ShotgunJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == SG5B) return(SetActionSmooth("ShotgunJump"));
  if (GetAction() eq "ShotgunWalk") if (GetID(Contents()) != SG5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "ShotgunJump") if (GetID(Contents()) != SG5B) return(SetActionSmooth("Jump"));

  if (GetAction() eq "Walk") if (GetID(Contents()) == HP5B) return(SetActionSmooth("HarpoonWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == HP5B) return(SetActionSmooth("HarpoonJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == HP5B) return(SetActionSmooth("HarpoonJump"));
  if (GetAction() eq "HarpoonWalk") if (GetID(Contents()) != HP5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "HarpoonJump") if (GetID(Contents()) != HP5B) return(SetActionSmooth("Jump"));
  if (GetAction() eq "HarpoonAim")
  {
    if (GetID(Contents()) != HP5B)
    {
      SetComDir(COMD_Stop());
      return(SetAction("Walk"));
    }
  }
  if (GetAction() eq "Walk") if (GetID(Contents()) == JP5B) return(SetActionSmooth("JetpackWalk"));
  if (GetAction() eq "Jump") if (GetID(Contents()) == JP5B) return(SetActionSmooth("JetpackJump"));
  if (GetAction() eq "Dive") if (GetID(Contents()) == JP5B) return(SetActionSmooth("JetpackJump"));
  if (GetAction() eq "JetpackWalk") if (GetID(Contents()) != JP5B) return(SetActionSmooth("Walk"));
  if (GetAction() eq "JetpackJump") if (GetID(Contents()) != JP5B) return(SetActionSmooth("Jump"));
  if (GetAction() eq "JetpackFly")
  {
    if (GetID(Contents()) == JP5B)
    {
      Contents() -> Flying(this());
      return(1);
    }
    SetComDir(COMD_Stop());
    SetAction("Jump");
  }
}

private func SetActionSmooth(newAction)
{
  var phase = GetPhase();
  SetAction(newAction);
  SetPhase(phase);
}

private func ScrollHud(mode, fast)
{  
  var owner = GetOwner(this());
  if (GetCursor(owner) != this()) return;

  var hud = FindObjectOwner(HU5B, owner);
  if(!hud)
  {
    // Halterung erzeugen
    CreateObject(MT5B, 0, 0, owner);    

    // Modus-Dreh-Bildchen erzeugen
    hud = CreateObject(HU5B, 0, 0, owner);
  }
  hud -> Scroll(mode, fast);
  return(1);
}

private func SetAmmoBar(int percent)
{
  ResetPhysical();
  DoMagicEnergy(0);

  SetPhysical("Magic", 100000, 2);
  DoMagicEnergy(percent);
}

private func Punching()
{
  if (!Random(3)) Sound("Kime*");
  
  Punch(GetActionTarget());
  Sound("Punch*");
  return(1);
}

private func Headbanging()
{
  if (!Random(3)) Sound("Kime*");

  var rCulprit = GetRank();
  var rVictim = GetRank(GetActionTarget());
  var rAdvantage = rCulprit - rVictim;

  if (rAdvantage >= 0)
  {
    DoEnergy(-rAdvantage, GetActionTarget());
    Punch(GetActionTarget());
  }
  else
  {
    DoEnergy(rAdvantage);
    Sound("Hurt*");
  }
  Sound("Punch*");
  return(1);
}

private func Kicking()
{
  if (!Random(3)) Sound("Kime*");
  
  DoEnergy(-10, GetActionTarget());
  Fling(GetActionTarget(), 4 * GetDir() - 2, -1);
  SetComDir(COMD_Stop()); 
  Sound("Punch*");
  return(1);
}

private func Chopping()
{
  if (GetActTime())
  {
    DoDamage(500, GetActionTarget());
  }
  else
  {
    Sound("Chop*");
  }
  return(1);
}

private func Building()
{
  if (GetAction() eq "Build")
  {
    // Funken und bläuliches Licht
    WeldingFX(20 * GetDir() - 10, 8);    

    // Maske ab, Schweißnaht checken
    if (!Random(10)) SetAction("WeldingMaskDown");
  }  
  else
  {
    // Maske auf, Schweißen!
    if (!Random(10)) SetAction("WeldingMaskUp");
  }
  return(1);
}

private func WeldingFX(xPos, yPos)
{
  CreateParticle("MSpark", xPos, yPos, RandomX(-15, 15), Random(15) * -1, 15, RGB(255, 255, 255));
  CreateParticle("MSpark", xPos, yPos, RandomX(-20, 20), Random(20) * -1, 15, RGB(255, 255, 255));
  CreateParticle("MSpark", xPos, yPos, RandomX(-25, 25), Random(25) * -1, 15, RGB(255, 255, 255));  

  CreateParticle("PSpark", xPos, yPos, RandomX(-1, 1), RandomX(-1, 1), RandomX(25, 50), RGB(Random(255), 255, 255));
  CreateParticle("PSpark", xPos, yPos, RandomX(-1, 1), RandomX(-1, 1), RandomX(100, 250), RGBa(Random(255), 255, 255, 150));
}

private func JetpackFX()
{
  var xPos = -14 * GetDir() + 7;
  var yPos = 1; 
 
  CreateParticle("PSpark", xPos, yPos, 0, 0, RandomX(25, 50), RGB(Random(255), 255, 255));
}

private func Holster()
{
  // Pistole wegwerfen, weiter mit Ejection()
  if (FindContents(PT5B)) return(Exit(pistol));

  if (!pistol)
  {
    pistol = CreateContents(PT5B);
    LocalN("ammo", pistol) = 100;
  }
  else
  {
    Enter(this(), pistol);
  }
  ShiftContents(0, true, PT5B);
  CheckArmed();
  Sound("SF_Holster");
  return(1);  
}

func FxSellCheckTimer()
{
  // weg, wenn Gebäude keine Heimatbasis ist
  // oder der Besitzer nicht genug Kohle hat;

  if (!pistol) return;

  var case = Contained();
  if (!case) return;

  var baseOwner = GetBase(case);
  if (baseOwner < 0)                     return;
  if (Hostile(baseOwner, GetOwner(), 1)) return;

  var ammo = LocalN("ammo", pistol);
  if (ammo == 100) return;

  var cashNeed = (100 - ammo) / 20;
  var cashHave = GetWealth(baseOwner);

  var ammoNeed = 100 - ammo;
  var ammoHave = cashHave * 20;

  ammo += Min(ammoNeed, ammoHave);
  LocalN("ammo", pistol) = ammo;
  if (Contained(pistol) == this()) SetAmmoBar(ammo);

  DoWealth(baseOwner, -Min(cashNeed, cashHave));
  if (Min(cashNeed, cashHave)) Sound("UnCash");
}

protected func ContentsDestruction()
{
  ScheduleCall(0, "CheckArmed", 1);
}

protected func RejectCollect()
{
  // nicht mehr als 5 Objekte tragbar
  if (ContentsCount() >= 5) return(1);
  return(0);
}

protected func Collection2(stuff)
{   
  if (ContentsCount(GetID(stuff)) == 1)
  {
    // hinten an die Inventarliste hängen
    if (GetID(stuff) != PT5B) ShiftContents();
  }
  // Bewaffnung prüfen
  CheckArmed();
  return(1);
}

protected func Ejection(stuff)
{
  if (GetID(stuff) == PT5B)
  {
    var hud = FindObjectOwner(HU5B, GetOwner());
    Enter(hud, pistol);
    Sound("SF_Holster");
  }
  // Bewaffnung prüfen
  CheckArmed();
  return(1);
}

protected func CrewSelection(deselect)
{
  var vis = VIS_None;

  if (deselect)
  {
    if (GetCrew(GetOwner()))
    {
      var nextClonk = GetCursor(GetOwner());
      if (GetID(nextClonk) == SF5B) vis = VIS_Owner;
    }
  }
  else
  {
    var object = Contents();
    if (GetAction() eq "AirbikeFly")
    {
      object = GetActionTarget();
    }
    ScrollHud(LocalN("mode", object), true);
    vis = VIS_Owner;
  }
  var hud    = FindObjectOwner(HU5B, GetOwner());
  var mount  = FindObjectOwner(MT5B, GetOwner());
  var biomap = FindObjectOwner(BM5B, GetOwner());
  
  if (hud)     SetVisibility(vis, hud);
  if (mount)   SetVisibility(vis, mount);
  if (biomap)  SetVisibility(vis, biomap);
}

protected func Death()
{
  // ggf. HUD entfernen
  CrewSelection(true);  

  // Info-Broadcasts für sterbende Clonks
  GameCallEx("OnClonkDeath", this(), GetKiller());
  
  // Der Broadcast könnte seltsame Dinge gemacht haben: Clonk ist noch tot?
  if (GetAlive()) return(0);
  
  Sound("SF_Die");
  DeathAnnounce();

  // Letztes Mannschaftsmitglied tot?
  if (!GetCrew(GetOwner())) GameCallEx("RelaunchPlayer", GetOwner(), this());

  // Tod dem Spiel(ziel) berichten
  GameCallEx("ReportHomicide", GetKiller(), GetOwner(), GetID(this));
  return(1);
}