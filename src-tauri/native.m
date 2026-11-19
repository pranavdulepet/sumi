#import <Foundation/Foundation.h>
#import <NaturalLanguage/NaturalLanguage.h>
#import <ApplicationServices/ApplicationServices.h>
#import <IOKit/ps/IOPowerSources.h>
#import <IOKit/ps/IOPSKeys.h>
#import <UserNotifications/UserNotifications.h>
#include <stdlib.h>
#include <pthread/qos.h>
void sumi_background_thread(void) {
    pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0);
}
int sumi_heavy_allowed(void) {
 @autoreleasepool {
   if ([NSProcessInfo processInfo].thermalState >= NSProcessInfoThermalStateSerious) return 0;
   CFTypeRef info=IOPSCopyPowerSourcesInfo();
   if(!info)return 0;
   CFStringRef power=IOPSGetProvidingPowerSourceType(info);
   BOOL onAC=power&&CFEqual(power,CFSTR(kIOPSACPowerValue));
   CFRelease(info);
   return onAC && CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState,kCGAnyInputEventType)>90.;
 }
}
char *sumi_entities(const char *input) {
 @autoreleasepool {
  NSString *s=[NSString stringWithUTF8String:input];
  if(!s)return strdup("[]");
  NLTagger *tagger=[[NLTagger alloc]initWithTagSchemes:@[NLTagSchemeNameType]];
  tagger.string=s;NSMutableArray *names=[NSMutableArray array];
  [tagger enumerateTagsInRange:NSMakeRange(0,s.length) unit:NLTokenUnitWord scheme:NLTagSchemeNameType options:NLTaggerOmitWhitespace|NLTaggerJoinNames usingBlock:^(NLTag tag,NSRange range,BOOL *stop){
    (void)stop; if([tag isEqualToString:NLTagPersonalName]) [names addObject:[s substringWithRange:range]];
  }];
  NSData *data=[NSJSONSerialization dataWithJSONObject:names options:0 error:nil];
  return strdup([[[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding] UTF8String]);
 }
}
void sumi_free(char *p){free(p);}
int sumi_semantic(const char *input,float *result,int length){
 @autoreleasepool{
  static NLEmbedding *embedding;static dispatch_once_t once;dispatch_once(&once,^{embedding=[NLEmbedding sentenceEmbeddingForLanguage:NLLanguageEnglish];});
  if(!embedding)return 0;NSString *s=[NSString stringWithUTF8String:input];NSArray<NSNumber*> *vec=[embedding vectorForString:s];
  if(!vec.count)return 0;
  for(NSUInteger i=0;i<vec.count;i++){result[i%length]+=[vec[i] floatValue];}return 1;
 }
}

static dispatch_group_t sumi_notification_permission(void) {
    static dispatch_group_t group;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ group = dispatch_group_create(); });
    return group;
}
void sumi_request_notifications(void) {
    @autoreleasepool {
        dispatch_group_t group = sumi_notification_permission();
        dispatch_group_enter(group);
        [[UNUserNotificationCenter currentNotificationCenter]
            requestAuthorizationWithOptions:UNAuthorizationOptionAlert
            completionHandler:^(BOOL granted, NSError *error) {
                (void)granted;
                (void)error;
                dispatch_group_leave(group);
            }];
    }
}
void sumi_notify_when_ready(void) {
    // A small notebook can finish before the person answers the permission sheet.
    dispatch_group_notify(sumi_notification_permission(), dispatch_get_main_queue(), ^{
        UNUserNotificationCenter *center = [UNUserNotificationCenter currentNotificationCenter];
        [center getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
            if (settings.authorizationStatus != UNAuthorizationStatusAuthorized &&
                settings.authorizationStatus != UNAuthorizationStatusProvisional) return;
            UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
            content.title = @"Sumi";
            content.body = @"Your first reading is ready.";
            UNNotificationRequest *request = [UNNotificationRequest
                requestWithIdentifier:@"first-reading" content:content trigger:nil];
            [center addNotificationRequest:request withCompletionHandler:nil];
        }];
    });
}
