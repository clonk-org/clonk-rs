/* Gate */

#strict

local command;

func Initialize() { SetAction("TryToOpen"); }

func Open()  { command = "open";  }
func Close() { command = "close"; }

func IsOpen()   { SetSolidMask();            }
func IsClosed() { SetSolidMask(0, 0, 49, 5); }

func TryToOpen()  { if (command == "open") SetAction("Open");   }
func TryToClose() { if (command == "close") SetAction("Close"); }