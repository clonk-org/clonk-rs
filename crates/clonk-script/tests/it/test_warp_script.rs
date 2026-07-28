// Test for WARP script patterns

#[test]
fn warp_script_simplified() {
    let source = r#"
private func Warp(clonk)
{
  var ox, oy;
  GetWarpPosition(ox, oy);

  if(ox == -1 && oy == -1) return(0, Message("No place", clonk) );

  Sound("Magic1");

  var dir = GetDir(clonk);
  if(dir == DIR_Left() ) dir = -1;

  var startwarp = CreateObject(WARP, BoundBy(65 * dir, -GetX(), LandscapeWidth() - GetX()), 10, -1);
  var endwarp = CreateObject(WARP, AbsX(ox), AbsY(oy), -1);

  startwarp->WARP::Connect(endwarp);

  RemoveObject();
  return(1);
}
"#;
    crate::support::assert_compiles(source);
}

#[test]
fn scope_resolution_call() {
    // Test the ::Connect pattern
    let source = r#"func Test() { var obj; obj->WARP::Connect(0); }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn comma_return_with_parentheses() {
    // Test return(0, Message(...))
    let source = r#"func Test() { return(0, Message("test")); }"#;
    crate::support::assert_compiles(source);
}
