using UnrealBuildTool;

public class VelaBattleEditorTarget : TargetRules
{
	public VelaBattleEditorTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Editor;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("VelaBattle");
	}
}
