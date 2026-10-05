using UnrealBuildTool;

public class VelaBattle : ModuleRules
{
	public VelaBattle(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		PublicDependencyModuleNames.AddRange(new string[]
		{
			"Core", "CoreUObject", "Engine", "InputCore", "BattleCore"
		});
	}
}
