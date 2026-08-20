// Test for WARP script patterns

crate::support::compile_cases! {
    warp_script_simplified:
    r#"
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

// Test the ::Connect pattern
    scope_resolution_call: r#"func Test() { var obj; obj->WARP::Connect(0); }"#;

// Test return(0, Message(...))
    comma_return_with_parentheses: r#"func Test() { return(0, Message("test")); }"#;
}
