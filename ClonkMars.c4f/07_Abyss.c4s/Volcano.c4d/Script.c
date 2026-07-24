/*-- Vulkan --*/

#strict

func Initialize() {
  CreateSound();
  return(1);
}

protected func CreateSound(){
  Sound("Effect_Vulcano",1,0,0,0,+1);
  return(1);
  }
