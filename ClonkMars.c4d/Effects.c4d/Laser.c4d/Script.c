/* Aus Hazard geklauter vereinfachter Laser für tollen Effekt! */

#strict

local w, l, r, c, iTime, iMaxDist, dx, dy;
local reflections;

protected func Initialize()
{
  SetObjectBlitMode(1);
  SetClrModulation(RGB(255,0,0));
}

/* Interface (public functions) */

public func SetAngle(int iAngle) {
   r = iAngle-180;
}
public func SetWidth(int iWidth) {
   w = 1000*iWidth/GetActMapVal("Facet","Laser",0,2);
}
public func SetMaxDistance(int iDist) {
   iMaxDist = iDist;
}

public func Set(int iAngle, int iWidth, int iDist, int iT)
{
  // optional parameters
  if(!iWidth) iWidth = 3;
  if(!iDist) iDist = 300;
  // iT = 0: no out-fading
  iTime = iT;

  // reset fade-timer
  c=0;

	SetAction("Laser");
  SetMaxDistance(iDist);
  SetAngle(iAngle);
  SetWidth(iWidth);
  l = 1000*iDist/GetActMapVal("Facet",GetAction(),0,3);
  DrawTransform();
  
 	// we are stuck?
	if(Stuck())
		RemoveObject();
}

public func GetAngle()  { return(r+180); }
public func GetWidth()  { return(w*GetActMapVal("Facet","Laser",0,2)/1000);  }
public func GetLength() { 
  if(GetAction()eq"Idle")
    return(l*GetActMapVal("Facet","Laser",0,3)/1000);
  else
    return(l*GetActMapVal("Facet",GetAction(),0,3)/1000);
}

public func GetMaxDistance() { return(iMaxDist); }

public func LaserEnd(&x, &y) {
  x = +Sin(GetAngle(),GetLength());
  y = -Cos(GetAngle(),GetLength());
}

protected func Laser() {		// fade out
  // animation & effect
  SetPhase(Random(4));
  if(GetAction() eq "Laser") {
    var x,y;
    LaserEnd(x,y);
    var color = GetClrModulation();
    var a,r,g,b;
    SplitRGBaValue(color,r,g,b,a);
    a = BoundBy(a+64,180,255);
    CreateParticle("PSpark",x,y,0,0,300+GetWidth()*30,RGBa(r,g,b,a));
  }

  if(!iTime) return();

  // fade out (only if time != 0)
  var a = 200*c/iTime;
  var rgba = SetRGBaValue(GetClrModulation(),a,0);
  SetClrModulation(rgba);
  ++c;

  if(c >= iTime)
    RemoveObject();
}

private func DrawTransform() {		// draw line

  var cl = l;
  cl = l*GetActMapVal("Facet","Laser",0,3)/GetActMapVal("Facet",GetAction(),0,3);

  var fsin = -Sin(r, 1000), fcos = Cos(r, 1000);

  var xoff = -(GetActMapVal("Facet",GetAction(),0,2)*w/1000)/2;
  var yoff = 0;

  var width = +fcos*w/1000, height = +fcos*cl/1000;
  var xskew = +fsin*cl/1000, yskew = -fsin*w/1000;

  var xadjust = +fcos*xoff + fsin*yoff;
  var yadjust = -fsin*xoff + fcos*yoff;

  // set matrix values
  SetObjDrawTransform (
    width, xskew, xadjust,
    yskew, height, yadjust
  );
  
}
