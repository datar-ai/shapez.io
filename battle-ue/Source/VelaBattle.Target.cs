using UnrealBuildTool;

public class VelaBattleTarget : TargetRules
{
	public VelaBattleTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Game;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("VelaBattle");
	}
}
