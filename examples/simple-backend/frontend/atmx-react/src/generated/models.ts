// GENERATED CODE – DO NOT EDIT.
/* eslint-disable @typescript-eslint/no-explicit-any */

/* eslint-disable @typescript-eslint/no-namespace */

export namespace pyExample {

  export interface Item {
    description?: string;
    id: string;
    ownerId: string;
    title: string;
  }
  

  export interface ItemCreate {
    description?: string;
    title: string;
  }
  

  export interface Token {
    accessToken: string;
    tokenType: string;
  }
  

  export interface User {
    email: any;
    id: string;
    role: string;
  }
  

  export interface UserCreate {
    email: any;
    password: string;
  }
  
}

export const Mappers: Record<string, any> = {
  pyExample: {
    Item: {
      fromJson: (json: any): pyExample.Item => ({
        description: json["description"],
        id: json["id"],
        ownerId: json["owner_id"],
        title: json["title"],
      }),
      toJson: (obj: any): any => ({
        "description": obj.description,
        "id": obj.id,
        "owner_id": obj.ownerId,
        "title": obj.title,
      })
    },
    ItemCreate: {
      fromJson: (json: any): pyExample.ItemCreate => ({
        description: json["description"],
        title: json["title"],
      }),
      toJson: (obj: any): any => ({
        "description": obj.description,
        "title": obj.title,
      })
    },
    Token: {
      fromJson: (json: any): pyExample.Token => ({
        accessToken: json["access_token"],
        tokenType: json["token_type"],
      }),
      toJson: (obj: any): any => ({
        "access_token": obj.accessToken,
        "token_type": obj.tokenType,
      })
    },
    User: {
      fromJson: (json: any): pyExample.User => ({
        email: json["email"],
        id: json["id"],
        role: json["role"],
      }),
      toJson: (obj: any): any => ({
        "email": obj.email,
        "id": obj.id,
        "role": obj.role,
      })
    },
    UserCreate: {
      fromJson: (json: any): pyExample.UserCreate => ({
        email: json["email"],
        password: json["password"],
      }),
      toJson: (obj: any): any => ({
        "email": obj.email,
        "password": obj.password,
      })
    },
  },
};
