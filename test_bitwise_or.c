/*-- Test --*/

func Test() {
  var arr = CreateArray(5);
  var i = 0;
  arr[i] = CreateObject(CLNK, 0, 0, 0);
  SetCategory(C4D_Object | C4D_Living, arr[i]);
}
